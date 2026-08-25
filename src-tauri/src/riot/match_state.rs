//! Pregame / coregame match parsing. Pure functions over the glz match payloads;
//! network fetching lives in `remote_api`. See spec §5.

use serde::Deserialize;
use serde_json::Value;

/// A player extracted from a pregame or coregame match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchPlayer {
    pub puuid: String,
    pub team: String,
    /// Agent uuid, lowercased. None when unselected (empty in pregame).
    pub character_id: Option<String>,
    /// Pregame only: "locked" | "selected". None in coregame.
    pub selection_state: Option<String>,
    pub account_level: u32,
    pub incognito: bool,
    pub hide_account_level: bool,
}

/// Parsed coregame (INGAME) match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoregameData {
    pub map_id: Option<String>,
    pub players: Vec<MatchPlayer>,
    /// The local player's team id (the team `own_puuid` is on).
    pub own_team: Option<String>,
}

/// Parsed pregame (agent select) match — only ever the local player's own team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PregameData {
    pub map_id: Option<String>,
    pub players: Vec<MatchPlayer>,
    pub own_team: Option<String>,
}

// --- wire structs -----------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct PlayerIdentityWire {
    #[serde(rename = "AccountLevel", default)]
    account_level: u32,
    #[serde(rename = "Incognito", default)]
    incognito: bool,
    #[serde(rename = "HideAccountLevel", default)]
    hide_account_level: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct PlayerWire {
    #[serde(rename = "Subject", default)]
    subject: String,
    #[serde(rename = "TeamID", default)]
    team_id: String,
    #[serde(rename = "CharacterID", default)]
    character_id: String,
    #[serde(rename = "CharacterSelectionState", default)]
    character_selection_state: String,
    #[serde(rename = "PlayerIdentity", default)]
    player_identity: Option<PlayerIdentityWire>,
}

impl PlayerWire {
    fn into_match_player(self, is_pregame: bool) -> MatchPlayer {
        let identity = self.player_identity;
        MatchPlayer {
            puuid: self.subject,
            team: self.team_id,
            character_id: normalize_character(&self.character_id),
            selection_state: if is_pregame && !self.character_selection_state.is_empty() {
                Some(self.character_selection_state)
            } else {
                None
            },
            account_level: identity.as_ref().map(|i| i.account_level).unwrap_or(0),
            incognito: identity.as_ref().map(|i| i.incognito).unwrap_or(false),
            hide_account_level: identity.map(|i| i.hide_account_level).unwrap_or(false),
        }
    }
}

/// Lowercase an agent uuid; empty -> None.
fn normalize_character(id: &str) -> Option<String> {
    if id.is_empty() {
        None
    } else {
        Some(id.to_lowercase())
    }
}

/// Extract a `MatchID` (coregame/pregame players endpoint) from `{"MatchID": "..."}`.
pub fn extract_match_id(value: &Value) -> Option<String> {
    value
        .get("MatchID")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

// --- coregame ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CoregameWire {
    #[serde(rename = "MapID", default)]
    map_id: String,
    #[serde(rename = "Players", default)]
    players: Vec<PlayerWire>,
}

/// Parse a coregame match. `own_team` is the team `own_puuid` is on. Consumes `value`
/// (no deep clone).
pub fn extract_coregame(value: Value, own_puuid: &str) -> CoregameData {
    let wire: CoregameWire = serde_json::from_value(value).unwrap_or(CoregameWire {
        map_id: String::new(),
        players: Vec::new(),
    });
    let players: Vec<MatchPlayer> =
        wire.players.into_iter().map(|p| p.into_match_player(false)).collect();
    let own_team = players
        .iter()
        .find(|p| p.puuid == own_puuid)
        .map(|p| p.team.clone());
    CoregameData {
        map_id: none_if_empty(wire.map_id),
        players,
        own_team,
    }
}

// --- pregame ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct PregameTeamWire {
    #[serde(rename = "TeamID", default)]
    team_id: String,
    #[serde(rename = "Players", default)]
    players: Vec<PlayerWire>,
}

#[derive(Debug, Deserialize)]
struct PregameWire {
    #[serde(rename = "MapID", default)]
    map_id: String,
    #[serde(rename = "AllyTeam", default)]
    ally_team: Option<PregameTeamWire>,
    #[serde(rename = "Teams", default)]
    teams: Vec<PregameTeamWire>,
}

/// Parse a pregame match. Only `AllyTeam.Players` exists during agent select — enemies
/// are never exposed here (hard platform limit). `own_team` comes from AllyTeam.TeamID,
/// falling back to searching `Teams` for `own_puuid`. Consumes `value` (no deep clone).
pub fn extract_pregame(value: Value, own_puuid: &str) -> PregameData {
    let wire: PregameWire = serde_json::from_value(value).unwrap_or(PregameWire {
        map_id: String::new(),
        ally_team: None,
        teams: Vec::new(),
    });

    // own_team: AllyTeam.TeamID first, else search Teams for the puuid.
    let own_team = wire
        .ally_team
        .as_ref()
        .map(|t| t.team_id.clone())
        .filter(|t| !t.is_empty())
        .or_else(|| {
            wire.teams
                .iter()
                .find(|team| team.players.iter().any(|p| p.subject == own_puuid))
                .map(|team| team.team_id.clone())
        });

    // Pregame players carry no per-player TeamID on the wire (that field is coregame-only);
    // the team id lives on the AllyTeam object. Stamp the derived own_team onto each player
    // so `is_ally` compares against the same value the snapshot publishes, even when
    // AllyTeam.TeamID was empty and own_team came from the Teams search. A per-player
    // TeamID, if Riot ever adds one, still wins.
    let players: Vec<MatchPlayer> = wire
        .ally_team
        .as_ref()
        .map(|t| {
            t.players
                .iter()
                .cloned()
                .map(|p| {
                    let mut player = p.into_match_player(true);
                    if player.team.is_empty() {
                        player.team = own_team.clone().unwrap_or_default();
                    }
                    player
                })
                .collect()
        })
        .unwrap_or_default();

    PregameData {
        map_id: none_if_empty(wire.map_id),
        players,
        own_team,
    }
}

fn none_if_empty(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_match_id() {
        assert_eq!(extract_match_id(&json!({ "MatchID": "abc" })).as_deref(), Some("abc"));
        assert_eq!(extract_match_id(&json!({ "MatchID": "" })), None);
        assert_eq!(extract_match_id(&json!({})), None);
    }

    #[test]
    fn coregame_full_roster_and_own_team() {
        let payload = json!({
            "MapID": "/Game/Maps/Ascent/Ascent",
            "Players": [
                { "Subject": "me", "TeamID": "Blue", "CharacterID": "ADD6443A-41BD-E414-F6AD-E58D267F4E95",
                  "PlayerIdentity": { "AccountLevel": 150, "Incognito": false, "HideAccountLevel": false } },
                { "Subject": "enemy", "TeamID": "Red", "CharacterID": "",
                  "PlayerIdentity": { "AccountLevel": 99, "Incognito": true, "HideAccountLevel": true } }
            ]
        });
        let data = extract_coregame(payload, "me");
        assert_eq!(data.map_id.as_deref(), Some("/Game/Maps/Ascent/Ascent"));
        assert_eq!(data.own_team.as_deref(), Some("Blue"));
        assert_eq!(data.players.len(), 2);
        // CharacterID lowercased.
        assert_eq!(data.players[0].character_id.as_deref(), Some("add6443a-41bd-e414-f6ad-e58d267f4e95"));
        assert_eq!(data.players[0].account_level, 150);
        // empty CharacterID -> None; incognito + hide flags carried.
        assert_eq!(data.players[1].character_id, None);
        assert!(data.players[1].incognito);
        assert!(data.players[1].hide_account_level);
        // coregame never sets selection_state.
        assert_eq!(data.players[0].selection_state, None);
    }

    #[test]
    fn coregame_missing_own_puuid_has_no_own_team() {
        let payload = json!({ "Players": [ { "Subject": "x", "TeamID": "Red" } ] });
        let data = extract_coregame(payload, "me");
        assert_eq!(data.own_team, None);
    }

    #[test]
    fn pregame_exposes_only_ally_team() {
        let payload = json!({
            "MapID": "/Game/Maps/Bonsai/Bonsai",
            "AllyTeam": {
                "TeamID": "Blue",
                "Players": [
                    { "Subject": "me", "TeamID": "Blue", "CharacterID": "",
                      "CharacterSelectionState": "",
                      "PlayerIdentity": { "AccountLevel": 200 } },
                    { "Subject": "mate", "TeamID": "Blue",
                      "CharacterID": "ADD6443A-41BD-E414-F6AD-E58D267F4E95",
                      "CharacterSelectionState": "locked",
                      "PlayerIdentity": { "AccountLevel": 88 } }
                ]
            },
            "Teams": [ { "TeamID": "Blue", "Players": [ { "Subject": "me" } ] } ]
        });
        let data = extract_pregame(payload, "me");
        assert_eq!(data.own_team.as_deref(), Some("Blue"));
        assert_eq!(data.players.len(), 2);
        // unselected agent -> None, selection state None (empty)
        assert_eq!(data.players[0].character_id, None);
        assert_eq!(data.players[0].selection_state, None);
        // locked mate
        assert_eq!(data.players[1].selection_state.as_deref(), Some("locked"));
        assert_eq!(data.players[1].character_id.as_deref(), Some("add6443a-41bd-e414-f6ad-e58d267f4e95"));
    }

    #[test]
    fn pregame_players_inherit_ally_team_id_when_absent_on_the_wire() {
        // The real pregame payload has NO per-player TeamID (coregame-only field); the id
        // lives on AllyTeam. Regression: every player was getting team "" and rendered as
        // an enemy. A per-player TeamID, when present, still wins.
        let payload = json!({
            "AllyTeam": {
                "TeamID": "Blue",
                "Players": [
                    { "Subject": "me", "CharacterID": "", "CharacterSelectionState": "" },
                    { "Subject": "mate", "CharacterID": "", "CharacterSelectionState": "",
                      "TeamID": "Red" }
                ]
            }
        });
        let data = extract_pregame(payload, "me");
        assert_eq!(data.own_team.as_deref(), Some("Blue"));
        assert_eq!(data.players[0].team, "Blue", "wire without TeamID inherits AllyTeam's");
        assert_eq!(data.players[1].team, "Red", "explicit per-player TeamID wins");
    }

    #[test]
    fn pregame_own_team_falls_back_to_teams_search() {
        let payload = json!({
            "AllyTeam": { "Players": [ { "Subject": "me", "TeamID": "Red" } ] },
            "Teams": [ { "TeamID": "Red", "Players": [ { "Subject": "me" } ] } ]
        });
        let data = extract_pregame(payload, "me");
        assert_eq!(data.own_team.as_deref(), Some("Red"));
    }
}
