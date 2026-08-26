//! Auto-updater for the portable exe: ask GitHub for the latest release, and swap the
//! running executable in place. There is no installer and no code signing, so integrity
//! rests on a SHA-256 published beside the exe and fetched over TLS from the same release.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The one release record both commands read. Unauthenticated (60 requests/h/IP), which
/// requires the repo and its releases to be public.
const LATEST_RELEASE: &str =
    "https://api.github.com/repos/Superamaja/valorant-lightweight-tracker/releases/latest";

/// GitHub rejects API requests without a User-Agent.
const USER_AGENT: &str = concat!("valorant-lightweight-tracker/", env!("CARGO_PKG_VERSION"));

/// The release asset names are version-free on purpose (see docs/release.md).
const EXE_ASSET: &str = "valorant-lightweight-tracker.exe";
const CHECKSUM_ASSET: &str = "valorant-lightweight-tracker.exe.sha256";

/// Extra extensions the swap uses: the download lands on `.exe.new`, the replaced binary
/// keeps running as `.exe.old` until the next start deletes it.
const NEW_SUFFIX: &str = "new";
const OLD_SUFFIX: &str = "old";

/// The running process still holds `.exe.old` for a moment after it spawns its replacement.
const CLEANUP_RETRY_DELAY: Duration = Duration::from_secs(3);

/// The checksum asset holds one bare digest, so anything bigger is not the file we asked for.
const CHECKSUM_MAX: usize = 256;

/// Every install failure leaves the current install usable, so the way out is always the same.
const MANUAL_DOWNLOAD: &str = "download the new version manually";

/// What a finished check tells the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// Whether `version` is newer than the running build.
    pub available: bool,
    /// The latest published version, without the tag's `v`.
    pub version: String,
}

/// One release and the two assets the updater needs from it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Release {
    version: String,
    exe_url: String,
    checksum_url: String,
}

// --- pure parsing / comparison ----------------------------------------------

/// Parse a `vX.Y.Z` tag into its three numbers. Anything else (pre-release suffix, a
/// missing field, a non-numeric part) is unusable and yields None.
fn parse_tag(tag: &str) -> Option<[u32; 3]> {
    let mut parts = tag.strip_prefix('v').unwrap_or(tag).split('.');
    let mut out = [0u32; 3];
    for slot in &mut out {
        *slot = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// Whether `latest` is a newer release than `current`. A version neither command can read is
/// an error rather than a quiet "nothing to offer" — it would otherwise hide a broken release,
/// and only a strictly newer tag is ever worth downloading.
fn is_newer(latest: &str, current: &str) -> Result<bool, String> {
    let l = parse_tag(latest)
        .ok_or_else(|| format!("the latest release is tagged \"{latest}\", which is not a version"))?;
    let c = parse_tag(current)
        .ok_or_else(|| format!("this build carries an unreadable version (\"{current}\")"))?;
    Ok(l > c)
}

/// Pull the version and both asset URLs out of a `releases/latest` payload. Both assets
/// must be present, or the release is not one this updater can install.
fn parse_release(body: &serde_json::Value) -> Option<Release> {
    let tag = body.get("tag_name")?.as_str()?;
    let version = tag.strip_prefix('v').unwrap_or(tag).to_string();
    let assets = body.get("assets")?.as_array()?;
    let url_of = |wanted: &str| {
        assets
            .iter()
            .find(|a| a.get("name").and_then(|n| n.as_str()) == Some(wanted))
            .and_then(|a| a.get("browser_download_url"))
            .and_then(|u| u.as_str())
            .map(String::from)
    };
    Some(Release {
        version,
        exe_url: url_of(EXE_ASSET)?,
        checksum_url: url_of(CHECKSUM_ASSET)?,
    })
}

/// Read the `.sha256` asset: exactly the 64 lowercase hex characters the release workflow
/// writes, with nothing around them. Anything else is not a digest this updater trusts.
fn parse_checksum(body: &[u8]) -> Option<&str> {
    let usable = body.len() == 64 && body.iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    usable
        .then_some(body)
        .and_then(|hex| std::str::from_utf8(hex).ok())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// `foo.exe` -> `foo.exe.new` / `foo.exe.old`. Appends rather than replaces, so the swap
/// never collides with a differently named neighbour.
fn sidecar(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.as_os_str().to_os_string();
    name.push(".");
    name.push(suffix);
    PathBuf::from(name)
}

// --- IO ---------------------------------------------------------------------

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        // Bounds a stalled transfer without capping the download's total duration.
        .read_timeout(Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| format!("could not start an HTTPS client: {e}"))
}

/// Fetch and parse the latest release. Offline, rate-limited (403/429) and shape-changed
/// responses all land here as a plain message the UI can show.
async fn fetch_release(client: &reqwest::Client) -> Result<Release, String> {
    let resp = client
        .get(LATEST_RELEASE)
        .send()
        .await
        .map_err(|e| format!("could not reach GitHub: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("GitHub answered {} for the latest release", status.as_u16()));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("could not read the release from GitHub: {e}"))?;
    parse_release(&body).ok_or_else(|| "the latest release carries no installable build".into())
}

/// Fetch the release's expected digest. The body is read under a cap, so a wrong or
/// substituted asset cannot pull an arbitrary download into memory.
async fn fetch_checksum(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("could not download the checksum: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("the checksum download answered {}", status.as_u16()));
    }

    let oversized = "the release's checksum file is not a checksum".to_string();
    if resp.content_length().is_some_and(|n| n as usize > CHECKSUM_MAX) {
        return Err(oversized);
    }
    let mut body = Vec::with_capacity(CHECKSUM_MAX);
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("could not read the checksum: {e}"))?
    {
        if body.len() + chunk.len() > CHECKSUM_MAX {
            return Err(oversized);
        }
        body.extend_from_slice(&chunk);
    }

    parse_checksum(&body)
        .map(String::from)
        .ok_or_else(|| "the release's checksum file is unreadable".to_string())
}

/// Download `url` to `dest`, hashing as it goes, and keep the file only when the digest
/// matches. Every handle is closed before this returns, so the caller may rename freely.
async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected: &str,
) -> Result<(), String> {
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("could not download the update: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("the update download answered {}", status.as_u16()));
    }

    let mut hasher = Sha256::new();
    let write = async {
        let mut file = std::fs::File::create(dest)?;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))?
        {
            hasher.update(&chunk);
            file.write_all(&chunk)?;
        }
        file.flush()?;
        file.sync_all()
    }
    .await;

    if let Err(e) = write {
        let _ = std::fs::remove_file(dest);
        return Err(format!("could not write the update next to the app: {e}"));
    }

    if hex_digest(&hasher.finalize()) != expected {
        let _ = std::fs::remove_file(dest);
        return Err("the downloaded update failed its checksum".into());
    }
    Ok(())
}

/// A step of the swap that failed while the install was still intact.
fn failed(step: &str, e: &std::io::Error) -> String {
    format!("could not {step} ({e})")
}

/// A swap that failed and could not undo itself: the app is missing from its own path until
/// the user renames the file back, so the message has to name that file.
fn stranded(step: &str, e: &std::io::Error, restore: &std::io::Error, old_exe: &Path) -> String {
    let name = old_exe
        .file_name()
        .unwrap_or(old_exe.as_os_str())
        .to_string_lossy();
    format!(
        "could not {step} ({e}) and could not put the app back ({restore}): the previous version \
         survives as {name} and has to be renamed back by hand"
    )
}

/// Undo one rename of the swap. A file another process momentarily holds (an antivirus scan
/// of the fresh download is the usual one) lets go straight away, so one retry is worth it.
fn undo_rename(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => std::fs::rename(from, to),
    }
}

/// Windows cannot overwrite a running exe but can rename it, so the swap is two renames
/// plus a detached relaunch. Every failure path either leaves a working install behind or
/// says which file to rename back, and the caller only exits once the replacement is running.
fn swap_and_relaunch(exe: &Path, new_exe: &Path, old_exe: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(old_exe);

    if let Err(e) = std::fs::rename(exe, old_exe) {
        let _ = std::fs::remove_file(new_exe);
        return Err(failed("set the running app aside", &e));
    }
    if let Err(e) = std::fs::rename(new_exe, exe) {
        if let Err(restore) = undo_rename(old_exe, exe) {
            return Err(stranded("put the new version in place", &e, &restore, old_exe));
        }
        let _ = std::fs::remove_file(new_exe);
        return Err(failed("put the new version in place", &e));
    }

    let mut command = Command::new(exe);
    if let Some(dir) = exe.parent() {
        command.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS: the replacement outlives this process instead of dying with it.
        command.creation_flags(0x0000_0008);
    }
    if let Err(e) = command.spawn() {
        // The new version is already installed; only put it aside if it can be moved away
        // cleanly, otherwise the previous binary has nowhere to go back to.
        if undo_rename(exe, new_exe).is_err() {
            return Err(format!(
                "could not start the new version ({e}): it is installed, so starting the app \
                 again runs it"
            ));
        }
        if let Err(restore) = undo_rename(old_exe, exe) {
            return Err(stranded("start the new version", &e, &restore, old_exe));
        }
        let _ = std::fs::remove_file(new_exe);
        return Err(failed("start the new version", &e));
    }
    Ok(())
}

/// Download, verify and install the latest release. Returns once the replacement process
/// is running — the caller then has to quit, or two copies are live.
pub async fn install_latest() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("could not locate the app: {e}"))?;
    let new_exe = sidecar(&exe, NEW_SUFFIX);
    let old_exe = sidecar(&exe, OLD_SUFFIX);

    let client = http()?;
    let release = fetch_release(&client).await?;
    if !is_newer(&release.version, env!("CARGO_PKG_VERSION"))? {
        return Err(format!(
            "the latest release (v{}) is not newer than this build",
            release.version
        ));
    }

    let checksum = fetch_checksum(&client, &release.checksum_url).await?;
    download_verified(&client, &release.exe_url, &new_exe, &checksum).await?;
    swap_and_relaunch(&exe, &new_exe, &old_exe)
}

/// Delete the binary an earlier update replaced. Best-effort and retried once, because the
/// process that spawned this one may still be exiting.
pub fn clean_previous_install() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let old_exe = sidecar(&exe, OLD_SUFFIX);
    if !old_exe.exists() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if std::fs::remove_file(&old_exe).is_err() {
            tokio::time::sleep(CLEANUP_RETRY_DELAY).await;
            let _ = std::fs::remove_file(&old_exe);
        }
    });
}

/// Ask GitHub whether a newer release exists.
#[tauri::command]
pub async fn check_update() -> Result<UpdateInfo, String> {
    let release = fetch_release(&http()?).await?;
    Ok(UpdateInfo {
        available: is_newer(&release.version, env!("CARGO_PKG_VERSION"))?,
        version: release.version,
    })
}

/// Install the latest release and restart into it. Every failure reaching the frontend ends
/// with the same advice, so the wording is settled here rather than at each failing step.
#[tauri::command]
pub async fn apply_update(app: tauri::AppHandle) -> Result<(), String> {
    install_latest()
        .await
        .map_err(|e| format!("{e} — {MANUAL_DOWNLOAD}"))?;
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_release_tags() {
        assert_eq!(parse_tag("v1.2.3"), Some([1, 2, 3]));
        assert_eq!(parse_tag("0.1.0"), Some([0, 1, 0]));
        // Anything but three plain numbers is not a release this updater installs.
        assert_eq!(parse_tag("v1.2"), None);
        assert_eq!(parse_tag("v1.2.3.4"), None);
        assert_eq!(parse_tag("v1.2.3-rc1"), None);
        assert_eq!(parse_tag("nightly"), None);
        assert_eq!(parse_tag(""), None);
    }

    #[test]
    fn compares_versions_field_by_field() {
        assert_eq!(is_newer("0.2.0", "0.1.9"), Ok(true));
        assert_eq!(is_newer("1.0.0", "0.99.99"), Ok(true));
        assert_eq!(is_newer("0.1.10", "0.1.9"), Ok(true)); // numeric, not lexicographic
        // Only strictly newer is installable: the same build and an older one both stay put.
        assert_eq!(is_newer("0.1.0", "0.1.0"), Ok(false));
        assert_eq!(is_newer("0.1.0", "0.2.0"), Ok(false));
    }

    #[test]
    fn an_unreadable_version_is_an_error_not_a_silent_no() {
        assert!(is_newer("nightly", "0.1.0").is_err());
        assert!(is_newer("v1.2.3-rc1", "0.1.0").is_err());
        assert!(is_newer("0.2.0", "garbage").is_err());
    }

    #[test]
    fn extracts_the_version_and_both_asset_urls() {
        let body = json!({
            "tag_name": "v0.2.0",
            "assets": [
                { "name": "valorant-lightweight-tracker.exe.sha256",
                  "browser_download_url": "https://x/sum" },
                { "name": "something-else.txt", "browser_download_url": "https://x/other" },
                { "name": "valorant-lightweight-tracker.exe",
                  "browser_download_url": "https://x/exe" }
            ]
        });
        let release = parse_release(&body).unwrap();
        assert_eq!(release.version, "0.2.0");
        assert_eq!(release.exe_url, "https://x/exe");
        assert_eq!(release.checksum_url, "https://x/sum");
    }

    #[test]
    fn a_release_missing_either_asset_is_not_installable() {
        let exe_only = json!({
            "tag_name": "v0.2.0",
            "assets": [
                { "name": "valorant-lightweight-tracker.exe",
                  "browser_download_url": "https://x/exe" }
            ]
        });
        assert_eq!(parse_release(&exe_only), None);
        assert_eq!(parse_release(&json!({ "assets": [] })), None);
        // A rate-limit body carries neither field.
        assert_eq!(parse_release(&json!({ "message": "API rate limit exceeded" })), None);
    }

    #[test]
    fn reads_the_checksum_file() {
        let digest = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        assert_eq!(parse_checksum(digest.as_bytes()), Some(digest));
        // Exactly the digest the release workflow writes — nothing around it, no upper case,
        // no sha256sum file name, and no trailing newline.
        assert_eq!(parse_checksum(format!("{digest}\n").as_bytes()), None);
        assert_eq!(parse_checksum(format!(" {digest}").as_bytes()), None);
        assert_eq!(parse_checksum(format!("{digest} *app.exe").as_bytes()), None);
        assert_eq!(parse_checksum(digest.to_ascii_uppercase().as_bytes()), None);
        assert_eq!(parse_checksum(b""), None);
        assert_eq!(parse_checksum(b"not-a-digest"), None);
        assert_eq!(parse_checksum(&digest.as_bytes()[..63]), None);
        assert_eq!(parse_checksum(format!("{}zz", &digest[..62]).as_bytes()), None);
    }

    #[test]
    fn hashes_to_lowercase_hex() {
        assert_eq!(
            hex_digest(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // The digest a checksum file carries is compared against exactly this text.
        assert_eq!(hex_digest(&[0x00, 0x0f, 0xff]), "000fff");
    }

    #[test]
    fn sidecars_sit_beside_the_exe() {
        let exe = Path::new(r"C:\apps\valorant-lightweight-tracker.exe");
        assert_eq!(
            sidecar(exe, NEW_SUFFIX),
            PathBuf::from(r"C:\apps\valorant-lightweight-tracker.exe.new")
        );
        assert_eq!(
            sidecar(exe, OLD_SUFFIX),
            PathBuf::from(r"C:\apps\valorant-lightweight-tracker.exe.old")
        );
    }
}
