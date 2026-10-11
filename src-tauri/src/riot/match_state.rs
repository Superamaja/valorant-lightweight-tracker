//! Pregame / coregame match parsing. Pure functions over the glz match payloads;
//! network fetching lives in `remote_api`. See spec §5.

use crate::riot::error::{Error, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::BTreeSet;

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
    /// Coregame only, in modes with more than two sides: the side this player is on (see
    /// `assign_groups`). None in two-team modes and in pregame.
    pub group: Option<u32>,
}

/// Parsed coregame (INGAME) match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoregameData {
    pub map_id: Option<String>,
    /// Raw `GamePodID` (the game server), e.g. `aresriot.aws-usw1-prod.na-gp-norcal-1`.
    pub game_pod_id: Option<String>,
    pub players: Vec<MatchPlayer>,
    /// The local player's team id (the team `own_puuid` is on).
    pub own_team: Option<String>,
}

/// Parsed pregame (agent select) match — only ever the local player's own team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PregameData {
    pub map_id: Option<String>,
    /// Raw `GamePodID`, when agent select already carries one.
    pub game_pod_id: Option<String>,
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

/// Deserialize a field leniently: a shape we don't understand becomes `None` (for
/// `PlayerIdentity`, the privacy-safe defaults below) instead of failing the whole roster.
fn lenient<'de, D, T>(d: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = Value::deserialize(d)?;
    Ok(serde_json::from_value(value).ok())
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
    #[serde(rename = "PlayerIdentity", default, deserialize_with = "lenient")]
    player_identity: Option<PlayerIdentityWire>,
    #[serde(rename = "TeamNumber", default, deserialize_with = "lenient")]
    team_number: Option<u32>,
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
            // A missing / unparseable PlayerIdentity degrades toward PRIVACY, never toward
            // exposure: unknown level, treated as incognito with a hidden level.
            account_level: identity.as_ref().map(|i| i.account_level).unwrap_or(0),
            incognito: identity.as_ref().map(|i| i.incognito).unwrap_or(true),
            hide_account_level: identity.map(|i| i.hide_account_level).unwrap_or(true),
            group: None,
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

/// The `CharacterSelectionState` of a player who has committed to an agent; the other live
/// value is `"selected"` (still picking, and still changeable).
const LOCKED: &str = "locked";

/// Whether every player of a pregame roster has locked an agent — the point past which the
/// pregame payload can no longer change, so nothing is left for a poll tick to discover. An
/// empty roster, a missing `CharacterSelectionState` and a lock without an agent id all count
/// as NOT locked, so the tick keeps running whenever the payload is unclear. Pure.
pub fn roster_fully_locked(players: &[MatchPlayer]) -> bool {
    !players.is_empty()
        && players
            .iter()
            .all(|p| p.selection_state.as_deref() == Some(LOCKED) && p.character_id.is_some())
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
    #[serde(rename = "GamePodID", default)]
    game_pod_id: String,
    #[serde(rename = "Players", default)]
    players: Vec<PlayerWire>,
}

/// Parse a coregame match. `own_team` is the team `own_puuid` is on. Consumes `value`
/// (no deep clone). A payload that does not deserialize, or that carries no usable players,
/// is an ERROR rather than an empty lobby — the caller retries instead of caching a
/// bogus "complete" match.
pub fn extract_coregame(value: Value, own_puuid: &str) -> Result<CoregameData> {
    let wire: CoregameWire = serde_json::from_value(value)?;
    let (mut players, team_numbers): (Vec<MatchPlayer>, Vec<Option<u32>>) = wire
        .players
        .into_iter()
        .filter(|p| !p.subject.is_empty())
        .map(|p| {
            let team_number = p.team_number;
            (p.into_match_player(false), team_number)
        })
        .unzip();
    assign_groups(&mut players, &team_numbers);
    if players.is_empty() {
        return Err(Error::MalformedPayload("coregame match has no players".into()));
    }
    let own_team = players
        .iter()
        .find(|p| p.puuid == own_puuid)
        .map(|p| p.team.clone());
    Ok(CoregameData {
        map_id: none_if_empty(wire.map_id),
        game_pod_id: none_if_empty(wire.game_pod_id),
        players,
        own_team,
    })
}

/// Group a roster that has more than two sides. `TeamNumber` is the side when every player
/// carries one and it takes more than two values (duo modes, where `TeamID` is only a
/// Blue/Red split); otherwise more than two distinct `TeamID`s are the sides, numbered by
/// their sorted order from 1. A two-sided roster gets no groups, so `TeamID` alone decides
/// who is an ally.
fn assign_groups(players: &mut [MatchPlayer], team_numbers: &[Option<u32>]) {
    let groups: Vec<u32> = match team_numbers.iter().copied().collect::<Option<Vec<u32>>>() {
        Some(numbers) if numbers.iter().collect::<BTreeSet<_>>().len() > 2 => numbers,
        _ => {
            let ids: Vec<&str> = players
                .iter()
                .map(|p| p.team.as_str())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            if ids.len() <= 2 {
                return;
            }
            players
                .iter()
                .map(|p| ids.partition_point(|id| *id < p.team.as_str()) as u32 + 1)
                .collect()
        }
    };
    for (player, group) in players.iter_mut().zip(groups) {
        player.group = Some(group);
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
    #[serde(rename = "GamePodID", default)]
    game_pod_id: String,
    #[serde(rename = "AllyTeam", default)]
    ally_team: Option<PregameTeamWire>,
    #[serde(rename = "Teams", default)]
    teams: Vec<PregameTeamWire>,
}

/// Parse a pregame match. Only `AllyTeam.Players` exists during agent select — enemies
/// are never exposed here (hard platform limit). `own_team` comes from AllyTeam.TeamID,
/// falling back to searching `Teams` for `own_puuid`. Consumes `value` (no deep clone).
/// A payload that does not deserialize, or that carries no usable ally players, is an ERROR
/// rather than an empty lobby.
pub fn extract_pregame(value: Value, own_puuid: &str) -> Result<PregameData> {
    let wire: PregameWire = serde_json::from_value(value)?;

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
                .filter(|p| !p.subject.is_empty())
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

    if players.is_empty() {
        return Err(Error::MalformedPayload("pregame match has no ally players".into()));
    }

    Ok(PregameData {
        map_id: none_if_empty(wire.map_id),
        game_pod_id: none_if_empty(wire.game_pod_id),
        players,
        own_team,
    })
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
            "GamePodID": "aresriot.aws-usw1-prod.na-gp-norcal-1",
            "Players": [
                { "Subject": "me", "TeamID": "Blue", "CharacterID": "ADD6443A-41BD-E414-F6AD-E58D267F4E95",
                  "PlayerIdentity": { "AccountLevel": 150, "Incognito": false, "HideAccountLevel": false } },
                { "Subject": "enemy", "TeamID": "Red", "CharacterID": "",
                  "PlayerIdentity": { "AccountLevel": 99, "Incognito": true, "HideAccountLevel": true } }
            ]
        });
        let data = extract_coregame(payload, "me").unwrap();
        assert_eq!(data.map_id.as_deref(), Some("/Game/Maps/Ascent/Ascent"));
        assert_eq!(data.game_pod_id.as_deref(), Some("aresriot.aws-usw1-prod.na-gp-norcal-1"));
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

    fn groups_of(players: Vec<Value>) -> Vec<Option<u32>> {
        let data = extract_coregame(json!({ "Players": players }), "p1").unwrap();
        data.players.iter().map(|p| p.group).collect()
    }

    #[test]
    fn duo_mode_groups_by_team_number_not_team_id() {
        // 8 duos: TeamNumber is the duo, TeamID only a lopsided Blue/Red split.
        let players = (1..=16u32)
            .map(|i| {
                let duo = i.div_ceil(2);
                let team = if duo == 2 { "Red" } else { "Blue" };
                json!({ "Subject": format!("p{i}"), "TeamID": team, "TeamNumber": duo })
            })
            .collect();
        let expected: Vec<Option<u32>> = (1..=16u32).map(|i| Some(i.div_ceil(2))).collect();
        assert_eq!(groups_of(players), expected);
    }

    #[test]
    fn free_for_all_groups_by_sorted_team_id_ordinal() {
        let players = ["c", "a", "b"]
            .iter()
            .enumerate()
            .map(|(i, team)| json!({ "Subject": format!("p{}", i + 1), "TeamID": team }))
            .collect();
        assert_eq!(groups_of(players), vec![Some(3), Some(1), Some(2)]);
    }

    #[test]
    fn two_team_rosters_get_no_groups() {
        let five_v_five = |with_team_number: bool| -> Vec<Value> {
            (1..=10)
                .map(|i| {
                    let (team, number) = if i <= 5 { ("Blue", 1) } else { ("Red", 2) };
                    let mut player = json!({ "Subject": format!("p{i}"), "TeamID": team });
                    if with_team_number {
                        player["TeamNumber"] = json!(number);
                    }
                    player
                })
                .collect()
        };
        assert!(groups_of(five_v_five(false)).iter().all(Option::is_none));
        assert!(groups_of(five_v_five(true)).iter().all(Option::is_none));
        // TeamNumber on only some players is not a usable grouping, and two TeamIDs are
        // the ordinary ally/enemy split.
        let partial = vec![
            json!({ "Subject": "p1", "TeamID": "Blue", "TeamNumber": 1 }),
            json!({ "Subject": "p2", "TeamID": "Blue", "TeamNumber": 2 }),
            json!({ "Subject": "p3", "TeamID": "Red" }),
        ];
        assert!(groups_of(partial).iter().all(Option::is_none));
    }

    #[test]
    fn coregame_missing_own_puuid_has_no_own_team() {
        let payload = json!({ "Players": [ { "Subject": "x", "TeamID": "Red" } ] });
        let data = extract_coregame(payload, "me").unwrap();
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
        let data = extract_pregame(payload, "me").unwrap();
        assert_eq!(data.own_team.as_deref(), Some("Blue"));
        assert_eq!(data.game_pod_id, None, "no GamePodID on the wire");
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
        let data = extract_pregame(payload, "me").unwrap();
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
        let data = extract_pregame(payload, "me").unwrap();
        assert_eq!(data.own_team.as_deref(), Some("Red"));
    }

    /// A pregame roster row with the given selection state + agent.
    fn ally(state: Option<&str>, agent: Option<&str>) -> MatchPlayer {
        MatchPlayer {
            puuid: "p".into(),
            team: "Blue".into(),
            character_id: agent.map(String::from),
            selection_state: state.map(String::from),
            account_level: 0,
            incognito: false,
            hide_account_level: false,
            group: None,
        }
    }

    #[test]
    fn a_roster_is_locked_only_when_every_ally_has_committed() {
        let locked = || ally(Some("locked"), Some("agent"));
        assert!(roster_fully_locked(&[locked(), locked()]));

        // One ally still picking keeps the roster open.
        assert!(!roster_fully_locked(&[locked(), ally(Some("selected"), Some("agent"))]));
        // No selection state at all is not a lock.
        assert!(!roster_fully_locked(&[locked(), ally(None, Some("agent"))]));
        // A lock without an agent id is a payload we can't trust as final.
        assert!(!roster_fully_locked(&[locked(), ally(Some("locked"), None)]));
        // An empty roster has nothing to have locked.
        assert!(!roster_fully_locked(&[]));
    }

    // --- malformed payloads must error, never become empty lobbies ----

    #[test]
    fn malformed_coregame_payloads_error() {
        // Wrong shape for a typed field -> serde failure.
        assert!(matches!(
            extract_coregame(json!({ "Players": "nope" }), "me"),
            Err(Error::Json(_))
        ));
        // Valid JSON, but no usable roster -> not a valid empty lobby.
        for payload in [json!({}), json!({ "Players": [] }), json!({ "Players": [ {} ] })] {
            assert!(
                matches!(extract_coregame(payload, "me"), Err(Error::MalformedPayload(_))),
                "a player-less coregame payload must not parse as an empty lobby"
            );
        }
    }

    #[test]
    fn malformed_pregame_payloads_error() {
        assert!(matches!(
            extract_pregame(json!({ "AllyTeam": 7 }), "me"),
            Err(Error::Json(_))
        ));
        for payload in [
            json!({}),
            json!({ "AllyTeam": { "TeamID": "Blue", "Players": [] } }),
            json!({ "Teams": [ { "TeamID": "Blue", "Players": [ { "Subject": "me" } ] } ] }),
        ] {
            assert!(
                matches!(extract_pregame(payload, "me"), Err(Error::MalformedPayload(_))),
                "a player-less pregame payload must not parse as an empty lobby"
            );
        }
    }

    #[test]
    fn an_absent_or_unreadable_player_identity_stays_private() {
        for player in [
            // No PlayerIdentity at all: treat the row as incognito with a hidden level rather
            // than exposing a name/level we cannot confirm the player agreed to show.
            json!({ "Subject": "me", "TeamID": "Blue" }),
            // A PlayerIdentity of an unexpected shape must not fail the whole roster, and must
            // not fall through to "exposed".
            json!({ "Subject": "me", "TeamID": "Blue", "PlayerIdentity": "garbage" }),
        ] {
            let data = extract_coregame(json!({ "Players": [player] }), "me").unwrap();
            assert_eq!(data.players.len(), 1);
            assert!(data.players[0].incognito);
            assert!(data.players[0].hide_account_level);
            assert_eq!(data.players[0].account_level, 0);
        }
    }

    #[test]
    fn subject_less_entries_are_skipped_not_rendered() {
        let payload = json!({
            "Players": [
                { "Subject": "", "TeamID": "Red" },
                { "Subject": "me", "TeamID": "Blue",
                  "PlayerIdentity": { "AccountLevel": 10, "Incognito": false, "HideAccountLevel": false } }
            ]
        });
        let data = extract_coregame(payload, "me").unwrap();
        assert_eq!(data.players.len(), 1);
        assert_eq!(data.players[0].puuid, "me");
        assert_eq!(data.own_team.as_deref(), Some("Blue"));
    }
}
