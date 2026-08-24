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
    /// competitivetiers `smallIcon`/`largeIcon` URL, or null (Unranked / unresolved).
    pub icon_url: Option<String>,
}

impl RankInfo {
    pub fn unranked() -> Self {
        Self { tier: 0, name: "Unranked".to_string(), icon_url: None }
    }
}

/// One row in the in-match player table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Account level, or null when hidden from this viewer.
    pub account_level: Option<u32>,
    /// Party grouping id (only set when the player is in a party of >1).
    pub party_id: Option<String>,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
