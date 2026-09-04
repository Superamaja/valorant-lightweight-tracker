//! Lockfile discovery + parsing + local basic-auth header building.
//!
//! Path: `%LOCALAPPDATA%\Riot Games\Riot Client\Config\lockfile`
//! Format: single line, colon-separated: `name:pid:port:password:protocol`.

use crate::riot::error::{Error, Result};
use base64::Engine;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lockfile {
    pub name: String,
    pub pid: u32,
    pub port: u16,
    pub password: String,
    pub protocol: String,
}

impl Lockfile {
    /// Parse the single-line lockfile body. Pure — testable from a fixture string.
    pub fn parse(contents: &str) -> Result<Self> {
        let line = contents.trim();
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() != 5 {
            return Err(Error::LockfileParse(format!(
                "expected 5 colon-separated fields, got {}",
                parts.len()
            )));
        }
        let pid = parts[1]
            .parse::<u32>()
            .map_err(|e| Error::LockfileParse(format!("bad pid: {e}")))?;
        let port = parts[2]
            .parse::<u16>()
            .map_err(|e| Error::LockfileParse(format!("bad port: {e}")))?;
        Ok(Lockfile {
            name: parts[0].to_string(),
            pid,
            port,
            password: parts[3].to_string(),
            protocol: parts[4].to_string(),
        })
    }

    /// `Authorization: Basic <base64("riot:" + password)>` header value.
    pub fn basic_auth_header(&self) -> String {
        let raw = format!("riot:{}", self.password);
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        format!("Basic {encoded}")
    }

    /// Base URL for the local client, e.g. `https://127.0.0.1:52995`.
    pub fn local_base_url(&self) -> String {
        format!("https://127.0.0.1:{}", self.port)
    }

    /// Local websocket URL, e.g. `wss://127.0.0.1:52995`.
    pub fn local_ws_url(&self) -> String {
        format!("wss://127.0.0.1:{}", self.port)
    }
}

/// The default lockfile path on Windows (`%LOCALAPPDATA%\Riot Games\...`).
pub fn default_path() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    let mut path = PathBuf::from(local);
    path.push("Riot Games");
    path.push("Riot Client");
    path.push("Config");
    path.push("lockfile");
    Some(path)
}

/// Whether the environment defines `%LOCALAPPDATA%` at all. A machine without it cannot have
/// a lockfile path to look at, which is a different report from "the client is not running".
pub fn local_app_data_set() -> bool {
    std::env::var_os("LOCALAPPDATA").is_some()
}

/// Read + parse the lockfile from the default path. `LockfileMissing` when the client
/// is not running (file absent) — a normal state, not a hard error.
pub fn read() -> Result<Lockfile> {
    let path = default_path().ok_or(Error::LockfileMissing)?;
    match std::fs::read_to_string(&path) {
        Ok(contents) => Lockfile::parse(&contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::LockfileMissing),
        Err(e) => Err(Error::Io(e)),
    }
}

/// True when the lockfile on disk still describes the client `current` was read from.
///
/// A fast Riot Client restart rewrites the SAME path with a new pid, port and password, so a
/// bare "does the path exist" check leaves a session talking to a dead endpoint forever.
/// Re-reading and comparing costs one small file read, cheap enough for the
/// reconnect cadence. A missing, unreadable or half-written lockfile is "not current" too —
/// the client is either gone or being replaced, and either way the session must restart.
pub fn still_current(current: &Lockfile) -> bool {
    read().is_ok_and(|disk| disk == *current)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Riot Client:23144:52995:Ss4WWtBoLIdaOoYm1FLKGw:https";

    #[test]
    fn parses_valid_lockfile() {
        let lf = Lockfile::parse(SAMPLE).unwrap();
        assert_eq!(lf.name, "Riot Client");
        assert_eq!(lf.pid, 23144);
        assert_eq!(lf.port, 52995);
        assert_eq!(lf.password, "Ss4WWtBoLIdaOoYm1FLKGw");
        assert_eq!(lf.protocol, "https");
    }

    #[test]
    fn trims_trailing_newline() {
        let lf = Lockfile::parse(&format!("{SAMPLE}\n")).unwrap();
        assert_eq!(lf.port, 52995);
    }

    #[test]
    fn rejects_wrong_field_count() {
        assert!(Lockfile::parse("a:b:c").is_err());
        assert!(Lockfile::parse("a:b:c:d:e:f").is_err());
    }

    #[test]
    fn rejects_bad_port() {
        assert!(Lockfile::parse("Riot Client:23144:notaport:pw:https").is_err());
    }

    #[test]
    fn builds_basic_auth_header() {
        let lf = Lockfile::parse(SAMPLE).unwrap();
        // base64("riot:Ss4WWtBoLIdaOoYm1FLKGw")
        let expected = base64::engine::general_purpose::STANDARD
            .encode(b"riot:Ss4WWtBoLIdaOoYm1FLKGw");
        assert_eq!(lf.basic_auth_header(), format!("Basic {expected}"));
    }

    #[test]
    fn credential_changes_make_a_different_lockfile() {
        // The staleness rule behind `still_current`: a restarted client keeps the path but
        // changes pid/port/password, and any one of those must read as a different client.
        let original = Lockfile::parse(SAMPLE).unwrap();
        assert_eq!(Lockfile::parse(SAMPLE).unwrap(), original);
        for changed in [
            "Riot Client:99999:52995:Ss4WWtBoLIdaOoYm1FLKGw:https", // new pid
            "Riot Client:23144:60001:Ss4WWtBoLIdaOoYm1FLKGw:https", // new port
            "Riot Client:23144:52995:0000000000000000000000:https", // new password
        ] {
            assert_ne!(Lockfile::parse(changed).unwrap(), original);
        }
    }

    #[test]
    fn builds_urls() {
        let lf = Lockfile::parse(SAMPLE).unwrap();
        assert_eq!(lf.local_base_url(), "https://127.0.0.1:52995");
        assert_eq!(lf.local_ws_url(), "wss://127.0.0.1:52995");
    }
}
