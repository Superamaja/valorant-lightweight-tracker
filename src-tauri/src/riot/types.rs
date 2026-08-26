//! Shared data types. The `TrackerSnapshot` and its children are the display-ready
//! shape serialized to the frontend (camelCase). See `docs/ipc-contract.md`.

use serde::{Deserialize, Serialize};

/// App status exposed to the UI. Exactly the four states the UI switches on.
/// Reconnection / "not ready" nuance is carried by `TrackerSnapshot::message`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppStatus {
    /// Lockfile missing, client not running, or connection lost.
    ValorantNotRunning,
    /// In the menus (no active match).
    Menus,
    /// Agent select (only own team visible).
    Pregame,
    /// Live match (full roster).
    Ingame,
}

/// The `sessionLoopState` value decoded from private presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLoopState {
    Menus,
    Pregame,
    Ingame,
}

impl SessionLoopState {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "MENUS" => Some(Self::Menus),
            "PREGAME" => Some(Self::Pregame),
            "INGAME" => Some(Self::Ingame),
            _ => None,
        }
    }
}

/// Static-data resolved agent info for a player.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    /// Agent uuid (lowercased).
    pub id: String,
    /// Display name (e.g. "Jett"), or empty if unresolved.
    pub name: String,
    /// `displayIcon` URL from valorant-api, or null if unresolved.
    pub icon_url: Option<String>,
}

/// Resolved rank (current or peak).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RankInfo {
    /// CompetitiveTier number (0 == Unranked).
    pub tier: u8,
    /// Human name from NUMBER_TO_RANK (e.g. "Immortal 2").
    pub name: String,
    /// competitivetiers `smallIcon`/`largeIcon` URL (set for Unranked too), or null when
    /// unresolved.
    pub icon_url: Option<String>,
}

impl RankInfo {
    pub fn unranked() -> Self {
        Self { tier: 0, name: "Unranked".to_string(), icon_url: None }
    }
}

/// Current-season win rate (phase 2). Derived from the MMR payload already fetched for
/// ranks — no extra request. `null` on the row when the player has 0 games this season.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WinRate {
    /// Wins / games * 100, rounded to an integer percent.
    pub percent: u32,
    /// Number of competitive games played this season (the "(14)" in vRY's "57 (14)").
    pub games: u32,
}

/// Outcome of one recent competitive match, derived from `RankedRatingEarned` sign.
/// A 0-RR match is genuinely ambiguous (vRY reads sign only) -> `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchResult {
    Win,
    Loss,
    Unknown,
}

/// A resolved weapon skin (phase 2). Populated only INGAME (loadouts aren't exposed in
/// pregame/menus).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinInfo {
    /// Skin display name (e.g. "Neptune Vandal"), empty if unresolved.
    pub name: String,
    /// valorant-api `displayIcon` URL, or null if unresolved.
    pub icon_url: Option<String>,
}

/// Which of a row's stat groups are still being fetched. A group is pending while the data
/// it needs has not settled; `false` everywhere means the row is final, so an absent value is
/// genuinely absent ("N/A") rather than in flight. Default = settled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingStats {
    /// The batch name resolution has not landed for this player.
    pub name: bool,
    /// The MMR payload has not landed: current rank, RR, leaderboard, peak, peak act and WR
    /// all come from it.
    pub rank: bool,
    /// competitiveupdates has not landed: ΔRR and the last-5 pips.
    pub history: bool,
    /// The recent match-details have not landed: HS% and KD.
    pub recent_stats: bool,
    /// The match's loadouts have not landed: Vandal and Phantom skins.
    pub skins: bool,
}

/// One row in the in-match player table.
// Not `Eq`: `kd` is a float. Snapshot dedup only ever needs `PartialEq`, and the value is a
// finite ratio (never NaN), so equality stays well behaved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRow {
    /// Opaque player id (puuid). Row key only.
    pub id: String,
    /// "GameName#TagLine", or null when hidden (incognito) / unresolved.
    pub name: Option<String>,
    /// Player has enabled streamer/incognito mode — UI should not de-anonymize.
    pub incognito: bool,
    /// Raw team id ("Red"/"Blue").
    pub team: String,
    /// True if on the local player's team.
    pub is_ally: bool,
    /// True if this row is the local player.
    pub is_self: bool,
    /// Resolved agent, or null (not yet selected in pregame / unresolved).
    pub agent: Option<AgentInfo>,
    /// Pregame only: "locked" | "selected" | null.
    pub agent_selection_state: Option<String>,
    /// Current competitive rank.
    pub current_rank: RankInfo,
    /// Ranked rating (0-100), 0 when unranked.
    pub rr: i32,
    /// Leaderboard position (nonzero only for Ascendant+ top players).
    pub leaderboard_rank: i32,
    /// Peak rank across all recorded seasons.
    pub peak_rank: RankInfo,
    /// Short label for the act the peak rank was achieved in ("E6: A3", "V26: A1"), or null
    /// when no season could be attributed to the peak.
    pub peak_rank_act: Option<String>,
    /// Account level, or null when hidden from this viewer.
    pub account_level: Option<u32>,
    /// Party grouping id (only set when the player is in a party of >1).
    pub party_id: Option<String>,
    /// Current-season win rate, or null when the player has 0 games this season.
    pub win_rate: Option<WinRate>,
    /// ΔRR of the player's newest competitive match, or null when no recent comp match.
    pub rr_change: Option<i32>,
    /// Up to 5 most recent competitive results, newest first (Win/Loss/Unknown). Empty
    /// when the player has no recent competitive matches.
    pub recent_results: Vec<MatchResult>,
    /// Headshot percentage across the last N recent competitive matches (0-100), or null
    /// when the player has no recent matches (vRY shows "N/a").
    pub headshot_percent: Option<u32>,
    /// Kills/deaths across the same recent competitive matches HS% is computed from,
    /// rounded to 2 decimals, or null when those matches carry no stats for the player.
    pub kd: Option<f64>,
    /// Equipped Vandal skin (INGAME only; null in pregame/menus).
    pub vandal_skin: Option<SkinInfo>,
    /// Equipped Phantom skin (INGAME only; null in pregame/menus).
    pub phantom_skin: Option<SkinInfo>,
    /// Which of this row's stat groups are still in flight.
    pub pending: PendingStats,
}

/// Resolved map info.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapInfo {
    /// Raw MapID path (e.g. "/Game/Maps/Ascent/Ascent").
    pub id: String,
    /// Display name ("Ascent"), or empty if unresolved.
    pub name: String,
    /// `splash` image URL, or null.
    pub splash_url: Option<String>,
    /// `listViewIcon` image URL, or null.
    pub list_view_url: Option<String>,
}

/// The full snapshot emitted to the frontend on every state change.
// Not `Eq` — it carries `PlayerRow`s (see the note there); dedup uses `PartialEq` only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerSnapshot {
    pub status: AppStatus,
    /// Resolved map (null in menus / not-running).
    pub map: Option<MapInfo>,
    /// Display mode name ("Competitive", "Custom", ...), null in menus.
    pub mode: Option<String>,
    /// Local player's team id (null outside a match).
    pub own_team: Option<String>,
    /// Player rows (empty in menus / not-running).
    pub players: Vec<PlayerRow>,
    /// Whether every per-player stat has settled. `false` on the progress snapshots of a
    /// match whose stats are still being fetched; `true` on the final snapshot of a match, on
    /// re-entry snapshots of an already-loaded match, and on all non-match states (Menus /
    /// ValorantNotRunning). Invariant: `enriched == true` implies no row carries a `pending`
    /// flag. The UI keys skeletons off the per-row `pending` groups, not off this flag.
    pub enriched: bool,
    /// Epoch milliseconds when this snapshot was produced.
    pub last_updated: u64,
    /// Optional human status line (e.g. "Attempting to reconnect...").
    pub message: Option<String>,
}

impl TrackerSnapshot {
    /// The default not-running snapshot.
    pub fn not_running(message: Option<String>) -> Self {
        Self {
            status: AppStatus::ValorantNotRunning,
            map: None,
            mode: None,
            own_team: None,
            players: Vec::new(),
            enriched: true,
            last_updated: now_millis(),
            message,
        }
    }
}

/// Current epoch milliseconds (0 if the clock is before the epoch, which never happens).
pub fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
