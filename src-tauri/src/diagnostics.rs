//! Release-build diagnostics: a small bounded record of what the tracker last saw at each
//! stage (lockfile, local API, own presence, remote, websocket), rendered as a plain-text
//! report the user copies into a GitHub issue. Not a log: one slot per category, overwritten.
//!
//! Everything reaching the report is either a fixed phrase, a count, or a value that has gone
//! through `describe_error` / `debug_log::short` — no token, password, private-presence blob,
//! full id, full URL or filesystem path can appear in it.

use crate::app_state::TrackerState;
use crate::riot::error::{Error, Result};
use crate::riot::lockfile::{self, Lockfile};
use crate::riot::types::{now_millis, TrackerSnapshot};
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::State;

/// Stand-in for a value the tracker never learned.
const DASH: &str = "-";

/// Stand-in for an environment fact this build cannot read (every non-Windows host).
const UNKNOWN: &str = "unknown";

/// The lockfile path is printed unexpanded on purpose: the expanded one carries the Windows
/// username.
const LOCKFILE_PATH: &str = r"%LOCALAPPDATA%\Riot Games\Riot Client\Config\lockfile";

/// Longest error detail the report carries. Long enough for a Riot error body, short enough
/// that one failure cannot drown the rest of the report.
const DETAIL_MAX_CHARS: usize = 160;

/// What the tracker last saw at each stage. One slot per category: a new observation
/// overwrites the old one, so the record stays a fixed size for the life of the process.
pub struct Diagnostics {
    pub started: Instant,
    /// Rebuild attempts since start.
    pub builds: u32,
    /// When `publish` last emitted a different status.
    pub status_since: Instant,
    pub lockfile: LockfileState,
    pub lockfile_at: Option<Instant>,
    /// Successful `connect()`s.
    pub connects: u32,
    /// The last successful connect, kept after the session is lost.
    pub session: Option<SessionDiag>,
    pub session_up: bool,
    pub local_error: Option<LastError>,
    pub presence: Option<PresenceDiag>,
    pub presence_at: Option<Instant>,
    pub not_ready_streak: u32,
    pub remote_error: Option<LastError>,
    pub remote_errors: u32,
    pub last_match: Option<MatchDiag>,
    pub ws: WsDiag,
}

impl Default for Diagnostics {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            builds: 0,
            status_since: now,
            lockfile: LockfileState::Unchecked,
            lockfile_at: None,
            connects: 0,
            session: None,
            session_up: false,
            local_error: None,
            presence: None,
            presence_at: None,
            not_ready_streak: 0,
            remote_error: None,
            remote_errors: 0,
            last_match: None,
            ws: WsDiag::default(),
        }
    }
}

/// How the last lockfile read went. Carries the port and protocol only — never the pid, the
/// password or the expanded path.
pub enum LockfileState {
    Unchecked,
    NoLocalAppData,
    Missing,
    Unreadable(String),
    ParseFailed(String),
    Ok { port: u16, protocol: String },
}

/// The last failure of one stage: which call it was, what it said, and when.
pub struct LastError {
    pub what: &'static str,
    pub detail: String,
    pub at: Instant,
}

/// What the connected session is talking to. Built once per connect.
pub struct SessionDiag {
    /// Region exactly as `/riotclient/region-locale` reported it.
    pub region_raw: String,
    /// Normalized region used for the glz host (differs only for `pbe`).
    pub region: String,
    pub shard: String,
    pub client_version: String,
    /// Whether `client_version` came from own presence rather than the valorant-api bootstrap.
    pub version_from_presence: bool,
    pub season_known: bool,
    pub static_complete: bool,
    pub static_version: String,
    pub own_puuid_short: String,
    pub since: Instant,
}

/// What the last presence read found. The blob itself is never kept — only its length.
pub struct PresenceDiag {
    pub total: usize,
    pub valorant: usize,
    pub own_found: bool,
    pub product: Option<String>,
    pub private_len: usize,
    pub decode_error: Option<String>,
    /// Untouched `sessionLoopState`, so a value the tracker does not recognize is still visible.
    pub session_state: Option<String>,
    pub queue_id: Option<String>,
    pub provisioning_flow: Option<String>,
    pub party_state: Option<String>,
}

/// The last match id the tracker resolved.
pub struct MatchDiag {
    pub id_short: String,
    pub ingame: bool,
    pub at: Instant,
}

/// The presence websocket's lifetime so far.
#[derive(Default)]
pub struct WsDiag {
    pub connects: u32,
    pub failures: u32,
    pub last_error: Option<LastError>,
    pub closed_at: Option<Instant>,
}

/// What the frontend knows and the backend does not.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiFacts {
    /// The waiting-screen line the user is looking at, or the match view.
    pub screen: String,
    /// Whether a finished match's table is retained and available to view.
    pub held_table: bool,
}

/// Gathered once per report rather than stored.
struct EnvFacts {
    app_version: &'static str,
    profile: &'static str,
    os: String,
    webview: String,
}

// --- recording ---------------------------------------------------------------

impl Diagnostics {
    /// Record the outcome of one lockfile read. `LockfileMissing` splits in two: a missing
    /// file (the client is not running) and an environment with no `%LOCALAPPDATA%` at all,
    /// which is a different bug report entirely.
    pub fn record_lockfile(&mut self, result: &Result<Lockfile>) {
        self.lockfile = match result {
            Ok(lf) => LockfileState::Ok { port: lf.port, protocol: lf.protocol.clone() },
            Err(Error::LockfileMissing) if !lockfile::local_app_data_set() => {
                LockfileState::NoLocalAppData
            }
            Err(Error::LockfileMissing) => LockfileState::Missing,
            Err(Error::LockfileParse(msg)) => LockfileState::ParseFailed(redact(msg)),
            Err(Error::Io(e)) => LockfileState::Unreadable(e.kind().to_string()),
            Err(err) => LockfileState::Unreadable(describe_error(err)),
        };
        self.lockfile_at = Some(Instant::now());
    }

    pub fn record_local_error(&mut self, what: &'static str, err: &Error) {
        self.local_error = Some(LastError::new(what, err));
    }

    pub fn record_remote_error(&mut self, what: &'static str, err: &Error) {
        self.remote_error = Some(LastError::new(what, err));
        self.remote_errors = self.remote_errors.saturating_add(1);
    }

    pub fn record_session(&mut self, session: SessionDiag) {
        self.session = Some(session);
        self.session_up = true;
        self.connects = self.connects.saturating_add(1);
    }

    pub fn session_lost(&mut self) {
        self.session_up = false;
    }

    /// The client version own presence reported, which outranks the valorant-api bootstrap.
    pub fn note_presence_version(&mut self, version: &str) {
        if let Some(session) = self.session.as_mut() {
            session.client_version = version.to_string();
            session.version_from_presence = true;
        }
    }

    /// A completed static-data top-up.
    pub fn note_static_data(&mut self, complete: bool, version: &str) {
        if let Some(session) = self.session.as_mut() {
            session.static_complete = complete;
            session.static_version = version.to_string();
        }
    }

    pub fn record_presence(&mut self, presence: PresenceDiag) {
        self.presence = Some(presence);
        self.presence_at = Some(Instant::now());
    }

    pub fn record_match(&mut self, id: &str, ingame: bool) {
        self.last_match = Some(MatchDiag {
            id_short: crate::debug_log::short(id).to_string(),
            ingame,
            at: Instant::now(),
        });
    }

    /// A listener that came up. Counted at the handshake rather than when the listener
    /// returns, so a connection that is still live is in the report.
    pub fn record_ws_connected(&mut self) {
        self.ws.connects = self.ws.connects.saturating_add(1);
    }

    /// How one listener lifetime ended: `Ok` is a connection that had come up and later
    /// closed, `Err` one that never came up.
    pub fn record_ws_closed(&mut self, result: &Result<()>) {
        match result {
            Ok(()) => self.ws.closed_at = Some(Instant::now()),
            Err(err) => {
                self.ws.failures = self.ws.failures.saturating_add(1);
                self.ws.last_error = Some(LastError::new("connect", err));
            }
        }
    }

    pub fn note_build(&mut self, not_ready_streak: u32) {
        self.builds = self.builds.saturating_add(1);
        self.not_ready_streak = not_ready_streak;
    }

    pub fn note_status_change(&mut self) {
        self.status_since = Instant::now();
    }
}

impl LastError {
    fn new(what: &'static str, err: &Error) -> Self {
        Self { what, detail: describe_error(err), at: Instant::now() }
    }
}

// --- pure formatting ---------------------------------------------------------

/// A short, safe description of an error. The ONLY path by which an `Error` reaches the
/// report: the variants that carry free text have it redacted, and the rest are spelled out
/// with the HTTP status / error code that identifies them in a bug report.
pub fn describe_error(err: &Error) -> String {
    match err {
        Error::LockfileMissing => "lockfile not found (Riot Client not running)".to_string(),
        Error::LockfileParse(msg) => format!("lockfile parse failed ({})", redact(msg)),
        Error::NotReady => "not ready (RPC_ERROR / 404 body)".to_string(),
        Error::BadClaims => "bad claims (401/403)".to_string(),
        Error::ResourceNotFound => "resource not found (404 / RESOURCE_NOT_FOUND)".to_string(),
        Error::RateLimited(Some(secs)) => format!("rate limited (retry-after {secs}s)"),
        Error::RateLimited(None) => "rate limited".to_string(),
        Error::Http(msg) => redact(msg),
        Error::WebSocket(msg) => format!("websocket: {}", redact(msg)),
        Error::MalformedPayload(msg) => format!("malformed payload: {}", redact(msg)),
        Error::Json(e) => format!("json: {}", redact(&e.to_string())),
        Error::Decode(msg) => format!("decode: {}", redact(msg)),
        Error::Io(e) => format!("io: {}", e.kind()),
    }
}

/// Strip anything id-bearing out of an error message: reqwest spells its transport failures
/// as `... for url (https://pd.eu.a.pvp.net/mmr/v1/players/<puuid>)`, so both the URL and the
/// phrase introducing it are cut, and what is left is bounded.
fn redact(msg: &str) -> String {
    let mut text = msg;
    if let Some(at) = find_scheme(text) {
        text = &text[..at];
    }
    if let Some(at) = text.find(" for url") {
        text = &text[..at];
    }
    cap(text.trim_end_matches(['(', ' ', ':', ',']))
}

/// Where the first URL starts, matched case-insensitively so an upper-cased scheme is caught
/// too. Only the two schemes an error message can carry are looked for.
fn find_scheme(msg: &str) -> Option<usize> {
    let lower = msg.to_ascii_lowercase();
    match (lower.find("http://"), lower.find("https://")) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (found, None) | (None, found) => found,
    }
}

/// The first `DETAIL_MAX_CHARS` characters, never splitting one.
fn cap(text: &str) -> String {
    text.chars().take(DETAIL_MAX_CHARS).collect()
}

/// How long ago `at` was, from `now`. Pure.
fn ago(now: Instant, at: Instant) -> String {
    let elapsed = now.saturating_duration_since(at);
    if elapsed < Duration::from_secs(1) {
        return "just now".to_string();
    }
    format!("{} ago", duration_text(elapsed))
}

/// A duration at the resolution a bug report needs: `42s`, `12m34s`, `3h05m`. Pure.
fn duration_text(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m{:02}s", secs / 60, secs % 60),
        _ => format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// `2026-09-03 14:22:05Z` for an epoch-milliseconds instant. Pure — the civil-from-days
/// conversion is a few lines of arithmetic, so no date crate is pulled in for one line of
/// output.
fn utc_timestamp(epoch_ms: u64) -> String {
    let secs = epoch_ms / 1000;
    let (year, month, day) = civil_from_days((secs / 86_400) as i64);
    let tod = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z",
        tod / 3600,
        (tod / 60) % 60,
        tod % 60
    )
}

/// Days since the Unix epoch -> (year, month, day), by Howard Hinnant's civil-from-days.
/// Pure.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// The caveat the two regions whose shard mapping was never verified against a live account
/// carry into every report they appear in (spec "Gaps not determined from vRY source"). Pure.
fn shard_note(region_raw: &str) -> &'static str {
    match region_raw {
        "latam" | "br" => " (inferred mapping, never live-verified)",
        _ => "",
    }
}

/// `1 connect` / `3 connects`. Pure.
fn connects_text(n: u32) -> String {
    format!("{n} connect{}", if n == 1 { "" } else { "s" })
}

/// A value the tracker may not have, rendered as `-` when it does not.
fn or_dash(value: Option<&str>) -> &str {
    value.filter(|v| !v.is_empty()).unwrap_or(DASH)
}

/// `none` or `<call> -> <detail>, <when>`.
fn error_text(now: Instant, error: Option<&LastError>) -> String {
    match error {
        None => "none".to_string(),
        Some(e) => format!("{} -> {}, {}", e.what, e.detail, ago(now, e.at)),
    }
}

/// Render the whole report. Deterministic: everything time-dependent is derived from `now`
/// and `wall_ms`, so the same record renders the same text.
fn render(
    d: &Diagnostics,
    snap: &TrackerSnapshot,
    ui: &UiFacts,
    env: &EnvFacts,
    now: Instant,
    wall_ms: u64,
) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("Valorant Lightweight Tracker diagnostics".to_string());
    lines.push(format!(
        "app: v{} ({}) | {} | {}",
        env.app_version, env.profile, env.os, env.webview
    ));
    lines.push(format!(
        "time: {} | uptime: {} | screen: \"{}\" | held last-match table available: {}",
        utc_timestamp(wall_ms),
        duration_text(now.saturating_duration_since(d.started)),
        ui.screen,
        if ui.held_table { "yes" } else { "no" }
    ));
    lines.push(format!(
        "status: {:?} (since {}) | message: {} | builds: {}",
        snap.status,
        ago(now, d.status_since),
        or_dash(snap.message.as_deref()),
        d.builds
    ));

    lines.push(String::new());
    lines.push("[lockfile]".to_string());
    lines.push(format!("path: {LOCKFILE_PATH}"));
    let checked = match d.lockfile_at {
        Some(at) => format!(", checked {}", ago(now, at)),
        None => String::new(),
    };
    lines.push(format!("result: {}{checked}", lockfile_text(&d.lockfile)));

    lines.push(String::new());
    lines.push("[local api]".to_string());
    lines.push(match &d.session {
        None => "session: never connected".to_string(),
        Some(s) => {
            let state = if d.session_up {
                format!(
                    "up for {} ({})",
                    duration_text(now.saturating_duration_since(s.since)),
                    connects_text(d.connects)
                )
            } else {
                format!("down (last up {}, {})", ago(now, s.since), connects_text(d.connects))
            };
            format!("session: {state} | own puuid: {}", s.own_puuid_short)
        }
    });
    lines.push(format!("last error: {}", error_text(now, d.local_error.as_ref())));

    lines.push(String::new());
    lines.push("[presence]".to_string());
    match &d.presence {
        None => lines.push("no presence read yet".to_string()),
        Some(p) => {
            let mut roster = format!(
                "roster: {} presences, {} valorant | own: {}",
                p.total,
                p.valorant,
                if p.own_found { "found" } else { "absent" }
            );
            if p.own_found {
                let decoded = match &p.decode_error {
                    None => "ok".to_string(),
                    Some(msg) => format!("failed ({msg})"),
                };
                roster.push_str(&format!(
                    " | product: {} | private: {} chars | decoded: {decoded}",
                    or_dash(p.product.as_deref()),
                    p.private_len
                ));
            }
            lines.push(roster);
            lines.push(format!(
                "sessionLoopState: {} | queueId: {} | provisioningFlow: {} | partyState: {}",
                or_dash(p.session_state.as_deref()),
                or_dash(p.queue_id.as_deref()),
                or_dash(p.provisioning_flow.as_deref()),
                or_dash(p.party_state.as_deref())
            ));
            let (version, source) = match &d.session {
                Some(s) if s.version_from_presence => {
                    (s.client_version.as_str(), " (from presence)")
                }
                Some(s) => (s.client_version.as_str(), " (from valorant-api)"),
                None => (DASH, ""),
            };
            lines.push(format!(
                "client version: {}{source} | updated {} | not-ready streak: {}",
                or_dash(Some(version)),
                match d.presence_at {
                    Some(at) => ago(now, at),
                    None => DASH.to_string(),
                },
                d.not_ready_streak
            ));
        }
    }

    lines.push(String::new());
    lines.push("[remote]".to_string());
    lines.push(match &d.session {
        None => format!("region: {DASH} (never connected)"),
        Some(s) => {
            let normalized = if s.region == s.region_raw {
                String::new()
            } else {
                format!(" (glz region {})", s.region)
            };
            format!(
                "region: {}{normalized} -> shard {}{} | season id: {} | static data: {} ({})",
                s.region_raw,
                s.shard,
                shard_note(&s.region_raw),
                if s.season_known { "known" } else { "unknown" },
                if s.static_complete { "complete" } else { "incomplete" },
                or_dash(Some(s.static_version.as_str()))
            )
        }
    });
    lines.push(match &d.last_match {
        None => "last match id: none resolved this session".to_string(),
        Some(m) => format!(
            "last match id: {} ({}) {}",
            m.id_short,
            if m.ingame { "ingame" } else { "pregame" },
            ago(now, m.at)
        ),
    });
    lines.push(format!(
        "last error: {} ({} total)",
        error_text(now, d.remote_error.as_ref()),
        d.remote_errors
    ));

    lines.push(String::new());
    lines.push("[websocket]".to_string());
    let closed = match d.ws.closed_at {
        Some(at) => format!(" | closed {}", ago(now, at)),
        None => String::new(),
    };
    lines.push(format!(
        "connects: {} | failures: {}{closed} | last error: {}",
        d.ws.connects,
        d.ws.failures,
        error_text(now, d.ws.last_error.as_ref())
    ));

    lines.join("\n")
}

/// The `result:` line's verdict, without its timestamp. Pure.
fn lockfile_text(state: &LockfileState) -> String {
    match state {
        LockfileState::Unchecked => "not checked yet".to_string(),
        LockfileState::NoLocalAppData => "LOCALAPPDATA is not set".to_string(),
        LockfileState::Missing => "missing (Riot Client not running?)".to_string(),
        LockfileState::Unreadable(msg) => format!("unreadable ({msg})"),
        LockfileState::ParseFailed(msg) => format!("parse failed ({msg})"),
        LockfileState::Ok { port, protocol } => format!("ok (port {port}, {protocol})"),
    }
}

// --- environment + command ---------------------------------------------------

/// What this build is running as. Read per report — none of it changes while the app runs,
/// but a report is produced once per user click.
fn env_facts() -> EnvFacts {
    EnvFacts {
        app_version: env!("CARGO_PKG_VERSION"),
        profile: if cfg!(debug_assertions) { "debug" } else { "release" },
        os: os_text(),
        webview: webview_text(),
    }
}

#[cfg(windows)]
fn os_text() -> String {
    let v = windows_version::OsVersion::current();
    format!("Windows {}.{} build {}", v.major, v.minor, v.build)
}

#[cfg(not(windows))]
fn os_text() -> String {
    UNKNOWN.to_string()
}

/// The installed WebView2 runtime. A missing runtime is exactly the kind of broken setup this
/// report exists for, so its absence is reported rather than hidden.
fn webview_text() -> String {
    tauri::webview_version().map_or_else(|_| UNKNOWN.to_string(), |v| format!("webview2 {v}"))
}

/// The preformatted report, for the frontend to put on the clipboard. Takes the snapshot lock
/// and the diagnostics lock one after the other, never nested — see `TrackerState`.
#[tauri::command]
pub fn get_diagnostics(state: State<'_, Arc<TrackerState>>, ui: UiFacts) -> String {
    let snap = state.snapshot();
    let diagnostics = state.diagnostics.lock().unwrap();
    render(&diagnostics, &snap, &ui, &env_facts(), Instant::now(), now_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::riot::types::AppStatus;

    /// 2026-09-03 14:22:05Z, the instant the documented example was taken at.
    const EXAMPLE_WALL_MS: u64 = 1_788_445_325_000;

    fn ago_of(now: Instant, secs: u64) -> Instant {
        now.checked_sub(Duration::from_secs(secs)).unwrap()
    }

    fn menus_snapshot() -> TrackerSnapshot {
        TrackerSnapshot {
            status: AppStatus::Menus,
            map: None,
            mode: None,
            own_team: None,
            players: Vec::new(),
            enriched: true,
            last_updated: EXAMPLE_WALL_MS,
            message: None,
        }
    }

    fn example_env() -> EnvFacts {
        EnvFacts {
            app_version: "0.1.2",
            profile: "release",
            os: "Windows 10.0 build 26100".to_string(),
            webview: "webview2 140.0.3485.54".to_string(),
        }
    }

    fn example_ui() -> UiFacts {
        UiFacts { screen: "Waiting for a match".to_string(), held_table: false }
    }

    /// The healthy steady state the report format was designed around.
    fn example_diagnostics(now: Instant) -> Diagnostics {
        Diagnostics {
            started: ago_of(now, 754),
            builds: 3,
            status_since: ago_of(now, 710),
            lockfile: LockfileState::Ok { port: 52995, protocol: "https".to_string() },
            lockfile_at: Some(ago_of(now, 712)),
            connects: 1,
            session: Some(SessionDiag {
                region_raw: "eu".to_string(),
                region: "eu".to_string(),
                shard: "eu".to_string(),
                client_version: "release-13.04-shipping-20-5340415".to_string(),
                version_from_presence: true,
                season_known: true,
                static_complete: true,
                static_version: "release-13.04-shipping-20-5340415".to_string(),
                own_puuid_short: "8f4c1d2e".to_string(),
                since: ago_of(now, 712),
            }),
            session_up: true,
            local_error: None,
            presence: Some(PresenceDiag {
                total: 14,
                valorant: 3,
                own_found: true,
                product: Some("valorant".to_string()),
                private_len: 612,
                decode_error: None,
                session_state: Some("MENUS".to_string()),
                queue_id: Some("competitive".to_string()),
                provisioning_flow: Some("Matchmaking".to_string()),
                party_state: Some("MATCHMAKING".to_string()),
            }),
            presence_at: Some(ago_of(now, 3)),
            not_ready_streak: 0,
            remote_error: None,
            remote_errors: 0,
            last_match: None,
            ws: WsDiag { connects: 1, failures: 0, last_error: None, closed_at: None },
        }
    }

    #[test]
    fn render_matches_the_documented_example() {
        let now = Instant::now();
        let report = render(
            &example_diagnostics(now),
            &menus_snapshot(),
            &example_ui(),
            &example_env(),
            now,
            EXAMPLE_WALL_MS,
        );
        let expected = "\
Valorant Lightweight Tracker diagnostics
app: v0.1.2 (release) | Windows 10.0 build 26100 | webview2 140.0.3485.54
time: 2026-09-03 14:22:05Z | uptime: 12m34s | screen: \"Waiting for a match\" | held last-match table available: no
status: Menus (since 11m50s ago) | message: - | builds: 3

[lockfile]
path: %LOCALAPPDATA%\\Riot Games\\Riot Client\\Config\\lockfile
result: ok (port 52995, https), checked 11m52s ago

[local api]
session: up for 11m52s (1 connect) | own puuid: 8f4c1d2e
last error: none

[presence]
roster: 14 presences, 3 valorant | own: found | product: valorant | private: 612 chars | decoded: ok
sessionLoopState: MENUS | queueId: competitive | provisioningFlow: Matchmaking | partyState: MATCHMAKING
client version: release-13.04-shipping-20-5340415 (from presence) | updated 3s ago | not-ready streak: 0

[remote]
region: eu -> shard eu | season id: known | static data: complete (release-13.04-shipping-20-5340415)
last match id: none resolved this session
last error: none (0 total)

[websocket]
connects: 1 | failures: 0 | last error: none";
        assert_eq!(report, expected);
    }

    #[test]
    fn a_fresh_state_renders_without_a_session() {
        let now = Instant::now();
        let report = render(
            &Diagnostics::default(),
            &TrackerSnapshot::not_running(Some("Waiting for Valorant...".into())),
            &example_ui(),
            &example_env(),
            now,
            EXAMPLE_WALL_MS,
        );
        assert!(report.contains("status: ValorantNotRunning (since just now) | message: Waiting for Valorant... | builds: 0"));
        assert!(report.contains("result: not checked yet"));
        assert!(!report.contains(", checked "));
        assert!(report.contains("session: never connected"));
        assert!(report.contains("no presence read yet"));
        assert!(report.contains("region: - (never connected)"));
        assert!(report.contains("last match id: none resolved this session"));
        assert!(report.contains("connects: 0 | failures: 0 | last error: none"));
    }

    #[test]
    fn lockfile_states_render_distinctly() {
        let rendered: Vec<String> = [
            LockfileState::Unchecked,
            LockfileState::NoLocalAppData,
            LockfileState::Missing,
            LockfileState::Unreadable("permission denied".to_string()),
            LockfileState::ParseFailed("expected 5 colon-separated fields, got 3".to_string()),
            LockfileState::Ok { port: 52995, protocol: "https".to_string() },
        ]
        .iter()
        .map(lockfile_text)
        .collect();
        assert_eq!(
            rendered,
            vec![
                "not checked yet",
                "LOCALAPPDATA is not set",
                "missing (Riot Client not running?)",
                "unreadable (permission denied)",
                "parse failed (expected 5 colon-separated fields, got 3)",
                "ok (port 52995, https)",
            ]
        );
    }

    #[test]
    fn a_missing_lockfile_reports_whether_localappdata_exists() {
        let mut d = Diagnostics::default();
        d.record_lockfile(&Err(Error::LockfileMissing));
        let expected = if lockfile::local_app_data_set() {
            "missing (Riot Client not running?)"
        } else {
            "LOCALAPPDATA is not set"
        };
        assert_eq!(lockfile_text(&d.lockfile), expected);
        assert!(d.lockfile_at.is_some());

        d.record_lockfile(&Ok(Lockfile::parse("Riot Client:23144:52995:secret:https").unwrap()));
        assert_eq!(lockfile_text(&d.lockfile), "ok (port 52995, https)");
    }

    #[test]
    fn describe_error_redacts_urls_and_caps_length() {
        assert_eq!(
            describe_error(&Error::Http(
                "error sending request for url (https://pd.eu.a.pvp.net/mmr/v1/players/8f4c1d2e-1111-2222-3333-444455556666)".into()
            )),
            "error sending request"
        );
        // An upper-cased scheme is cut just the same, and so is a bare URL with no lead-in.
        assert_eq!(
            describe_error(&Error::Http("connect failed: HTTPS://GLZ-EU-1.EU.A.PVP.NET/x".into())),
            "connect failed"
        );
        // The fixed phrases carry the status/code a bug report is triaged by.
        assert_eq!(describe_error(&Error::BadClaims), "bad claims (401/403)");
        assert_eq!(describe_error(&Error::NotReady), "not ready (RPC_ERROR / 404 body)");
        assert_eq!(
            describe_error(&Error::ResourceNotFound),
            "resource not found (404 / RESOURCE_NOT_FOUND)"
        );
        assert_eq!(describe_error(&Error::RateLimited(Some(12))), "rate limited (retry-after 12s)");
        assert_eq!(describe_error(&Error::RateLimited(None)), "rate limited");
        // The local API's own message keeps its path and status, which is the useful part.
        assert_eq!(
            describe_error(&Error::Http("local /entitlements/v1/token -> 503".into())),
            "local /entitlements/v1/token -> 503"
        );
        // Length is bounded, and the cut never splits a character.
        let long = describe_error(&Error::Http("é".repeat(400)));
        assert_eq!(long.chars().count(), DETAIL_MAX_CHARS);
    }

    #[test]
    fn ids_are_truncated_to_eight_characters() {
        let mut d = Diagnostics::default();
        d.record_match("a1b2c3d4-5555-6666-7777-888899990000", true);
        let m = d.last_match.as_ref().unwrap();
        assert_eq!(m.id_short, "a1b2c3d4");
        assert!(m.ingame);
    }

    #[test]
    fn report_never_contains_secrets() {
        const PASSWORD: &str = "Ss4WWtBoLIdaOoYm1FLKGw";
        const PRIVATE_BLOB: &str = "eyJzZXNzaW9uTG9vcFN0YXRlIjoiTUVOVVMifQ==";
        const PUUID: &str = "8f4c1d2e-1111-2222-3333-444455556666";
        const MATCH_ID: &str = "a1b2c3d4-5555-6666-7777-888899990000";

        let now = Instant::now();
        let mut d = example_diagnostics(now);
        d.record_lockfile(&Ok(Lockfile::parse(&format!(
            "Riot Client:23144:52995:{PASSWORD}:https"
        ))
        .unwrap()));
        d.record_match(MATCH_ID, true);
        d.record_presence(PresenceDiag {
            total: 14,
            valorant: 3,
            own_found: true,
            product: Some("valorant".to_string()),
            private_len: PRIVATE_BLOB.len(),
            decode_error: None,
            session_state: Some("MENUS".to_string()),
            queue_id: None,
            provisioning_flow: None,
            party_state: None,
        });
        d.record_remote_error(
            "mmr",
            &Error::Http(format!(
                "error sending request for url (https://pd.eu.a.pvp.net/mmr/v1/players/{PUUID})"
            )),
        );
        d.session.as_mut().unwrap().own_puuid_short =
            crate::debug_log::short(PUUID).to_string();

        let report = render(
            &d,
            &menus_snapshot(),
            &UiFacts { screen: "Loading the lobby".to_string(), held_table: true },
            &example_env(),
            now,
            EXAMPLE_WALL_MS,
        );
        for secret in [PASSWORD, PRIVATE_BLOB, PUUID, MATCH_ID] {
            assert!(!report.contains(secret), "report leaked {secret}:\n{report}");
        }
        assert!(!report.contains("http://") && !report.contains("https://"));
        assert!(!report.contains("pvp.net"));
        // The truncated forms are what remains.
        assert!(report.contains("own puuid: 8f4c1d2e"));
        assert!(report.contains("last match id: a1b2c3d4 (ingame)"));
    }

    #[test]
    fn remote_errors_keep_only_the_last_but_count_all() {
        let now = Instant::now();
        let mut d = example_diagnostics(now);
        d.record_remote_error("match-id (glz pregame)", &Error::ResourceNotFound);
        d.record_remote_error("mmr", &Error::RateLimited(Some(6)));
        d.record_remote_error("match-id (glz core-game)", &Error::ResourceNotFound);
        assert_eq!(d.remote_errors, 3);

        let report = render(&d, &menus_snapshot(), &example_ui(), &example_env(), now, EXAMPLE_WALL_MS);
        assert!(report.contains(
            "last error: match-id (glz core-game) -> resource not found (404 / RESOURCE_NOT_FOUND), just now (3 total)"
        ));
        assert!(!report.contains("rate limited"));
    }

    #[test]
    fn latam_and_br_carry_the_inferred_shard_note() {
        assert_eq!(shard_note("br"), " (inferred mapping, never live-verified)");
        assert_eq!(shard_note("latam"), " (inferred mapping, never live-verified)");

        let now = Instant::now();
        let mut d = example_diagnostics(now);
        let session = d.session.as_mut().unwrap();
        session.region_raw = "br".to_string();
        session.region = "br".to_string();
        session.shard = "na".to_string();
        let report = render(&d, &menus_snapshot(), &example_ui(), &example_env(), now, EXAMPLE_WALL_MS);
        assert!(report.contains(
            "region: br -> shard na (inferred mapping, never live-verified) | season id: known"
        ));
    }

    #[test]
    fn other_regions_do_not() {
        for region in ["na", "eu", "ap", "kr", "pbe"] {
            assert_eq!(shard_note(region), "");
        }
        // pbe is the one region whose glz host differs from what region-locale reported.
        let now = Instant::now();
        let mut d = example_diagnostics(now);
        let session = d.session.as_mut().unwrap();
        session.region_raw = "pbe".to_string();
        session.region = "na".to_string();
        session.shard = "na".to_string();
        let report = render(&d, &menus_snapshot(), &example_ui(), &example_env(), now, EXAMPLE_WALL_MS);
        assert!(report.contains("region: pbe (glz region na) -> shard na |"));
    }

    #[test]
    fn ago_and_duration_formatting() {
        let now = Instant::now();
        assert_eq!(ago(now, now), "just now");
        assert_eq!(ago(now, ago_of(now, 42)), "42s ago");
        assert_eq!(ago(now, ago_of(now, 754)), "12m34s ago");
        assert_eq!(ago(now, ago_of(now, 11_100)), "3h05m ago");
        // A clock that appears to run backwards reads as "just now", never panics.
        assert_eq!(ago(ago_of(now, 60), now), "just now");

        assert_eq!(duration_text(Duration::from_secs(0)), "0s");
        assert_eq!(duration_text(Duration::from_secs(59)), "59s");
        assert_eq!(duration_text(Duration::from_secs(60)), "1m00s");
        assert_eq!(duration_text(Duration::from_secs(3599)), "59m59s");
        assert_eq!(duration_text(Duration::from_secs(3600)), "1h00m");
    }

    #[test]
    fn utc_timestamp_known_values() {
        assert_eq!(utc_timestamp(0), "1970-01-01 00:00:00Z");
        assert_eq!(utc_timestamp(999), "1970-01-01 00:00:00Z");
        assert_eq!(utc_timestamp(EXAMPLE_WALL_MS), "2026-09-03 14:22:05Z");
        // A leap day and the turn of a century-leap year.
        assert_eq!(utc_timestamp(1_709_164_800_000), "2024-02-29 00:00:00Z");
        assert_eq!(utc_timestamp(951_782_400_000), "2000-02-29 00:00:00Z");
    }

    #[test]
    fn a_lost_session_keeps_what_it_was_connected_to() {
        let now = Instant::now();
        let mut d = example_diagnostics(now);
        d.connects = 2;
        d.session_lost();
        let report = render(&d, &menus_snapshot(), &example_ui(), &example_env(), now, EXAMPLE_WALL_MS);
        assert!(report.contains("session: down (last up 11m52s ago, 2 connects) | own puuid: 8f4c1d2e"));
        assert!(report.contains("region: eu -> shard eu"));
    }

    #[test]
    fn an_unreadable_own_presence_is_reported_without_its_blob() {
        let now = Instant::now();
        let mut d = example_diagnostics(now);
        d.record_presence(PresenceDiag {
            total: 14,
            valorant: 3,
            own_found: true,
            product: Some("valorant".to_string()),
            private_len: 0,
            decode_error: Some(describe_error(&Error::NotReady)),
            session_state: None,
            queue_id: None,
            provisioning_flow: None,
            party_state: None,
        });
        d.note_build(2);
        let report = render(&d, &menus_snapshot(), &example_ui(), &example_env(), now, EXAMPLE_WALL_MS);
        assert!(report.contains(
            "private: 0 chars | decoded: failed (not ready (RPC_ERROR / 404 body))"
        ));
        assert!(report.contains("sessionLoopState: - | queueId: - | provisioningFlow: - | partyState: -"));
        assert!(report.contains("not-ready streak: 2"));
        assert!(report.contains("builds: 4"));
    }

    #[test]
    fn an_absent_own_presence_drops_the_rest_of_its_line() {
        let now = Instant::now();
        let mut d = example_diagnostics(now);
        d.record_presence(PresenceDiag {
            total: 14,
            valorant: 0,
            own_found: false,
            product: None,
            private_len: 0,
            decode_error: None,
            session_state: None,
            queue_id: None,
            provisioning_flow: None,
            party_state: None,
        });
        let report = render(&d, &menus_snapshot(), &example_ui(), &example_env(), now, EXAMPLE_WALL_MS);
        assert!(report.contains("roster: 14 presences, 0 valorant | own: absent\n"));
        assert!(!report.contains("decoded:"));
    }

    #[test]
    fn a_websocket_that_dropped_reports_when_and_why() {
        let now = Instant::now();
        let mut d = example_diagnostics(now);
        d.record_ws_connected();
        d.record_ws_closed(&Ok(()));
        d.record_ws_closed(&Err(Error::WebSocket("connection refused".into())));
        let report = render(&d, &menus_snapshot(), &example_ui(), &example_env(), now, EXAMPLE_WALL_MS);
        assert!(report.contains(
            "connects: 2 | failures: 1 | closed just now | last error: connect -> websocket: connection refused, just now"
        ));
    }
}
