//! Phase 2 coregame loadout parsing: extract each player's equipped Vandal/Phantom skin
//! uuid from `/core-game/v1/matches/{id}/loadouts`. Pure and fixture-testable; skin uuid ->
//! name/icon resolution happens later against `StaticData`. See spec Live verification 2.

use crate::riot::constants::{PHANTOM_WEAPON_ID, SKIN_SOCKET_ID, VANDAL_WEAPON_ID};
use serde_json::Value;
use std::collections::HashMap;

/// The equipped skin uuids for the two rifles we surface. `None` for a weapon the player
/// has no loadout entry for (shouldn't happen for base rifles, but handled defensively).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayerSkinIds {
    pub vandal: Option<String>,
    pub phantom: Option<String>,
}

/// Read the equipped skin uuid for `weapon_id` from a loadout's `Items` map:
/// `Items[weapon_id].Sockets[SKIN_SOCKET_ID].Item.ID`.
fn skin_id_for(items: &Value, weapon_id: &str) -> Option<String> {
    items
        .get(weapon_id)?
        .get("Sockets")?
        .get(SKIN_SOCKET_ID)?
        .get("Item")?
        .get("ID")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Parse a loadouts payload into puuid -> {vandal, phantom} skin uuids. Consumes `value`
/// (borrowed here; the payload is small — ~79 KB for a full lobby). Never fails.
pub fn parse_loadouts(value: &Value) -> HashMap<String, PlayerSkinIds> {
    let mut out = HashMap::new();
    let Some(loadouts) = value.get("Loadouts").and_then(|l| l.as_array()) else {
        return out;
    };
    for entry in loadouts {
        let Some(subject) = entry.get("Subject").and_then(|s| s.as_str()) else {
            continue;
        };
        if subject.is_empty() {
            continue;
        }
        // `Loadout.Items` (verified nesting: entry -> Loadout -> Items).
        let Some(items) = entry.get("Loadout").and_then(|l| l.get("Items")) else {
            continue;
        };
        out.insert(
            subject.to_string(),
            PlayerSkinIds {
                vandal: skin_id_for(items, VANDAL_WEAPON_ID),
                phantom: skin_id_for(items, PHANTOM_WEAPON_ID),
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_vandal_and_phantom_skin_ids() {
        // Sanitized from the live coregame-loadouts capture: one player with a Vandal and
        // a Phantom skin equipped, plus an unrelated weapon that must be ignored.
        let payload = json!({ "Loadouts": [
            { "Subject": "player-a", "CharacterID": "agent-x", "Loadout": { "Items": {
                VANDAL_WEAPON_ID: { "ID": VANDAL_WEAPON_ID, "Sockets": {
                    SKIN_SOCKET_ID: { "ID": SKIN_SOCKET_ID, "Item": { "ID": "skin-neptune-vandal" } },
                    "other-socket": { "Item": { "ID": "ignore-me" } }
                }},
                PHANTOM_WEAPON_ID: { "ID": PHANTOM_WEAPON_ID, "Sockets": {
                    SKIN_SOCKET_ID: { "ID": SKIN_SOCKET_ID, "Item": { "ID": "skin-reaver-phantom" } }
                }},
                "some-pistol": { "Sockets": {
                    SKIN_SOCKET_ID: { "Item": { "ID": "skin-classic" } }
                }}
            }}}
        ]});
        let map = parse_loadouts(&payload);
        let a = map.get("player-a").unwrap();
        assert_eq!(a.vandal.as_deref(), Some("skin-neptune-vandal"));
        assert_eq!(a.phantom.as_deref(), Some("skin-reaver-phantom"));
    }

    #[test]
    fn missing_weapon_yields_none() {
        // Player only has a Vandal in their loadout — Phantom absent.
        let payload = json!({ "Loadouts": [
            { "Subject": "p", "Loadout": { "Items": {
                VANDAL_WEAPON_ID: { "Sockets": {
                    SKIN_SOCKET_ID: { "Item": { "ID": "van-skin" } }
                }}
            }}}
        ]});
        let map = parse_loadouts(&payload);
        let p = map.get("p").unwrap();
        assert_eq!(p.vandal.as_deref(), Some("van-skin"));
        assert_eq!(p.phantom, None);
    }

    #[test]
    fn empty_or_malformed_is_empty_map() {
        assert!(parse_loadouts(&json!({})).is_empty());
        assert!(parse_loadouts(&json!({ "Loadouts": [] })).is_empty());
        // Entry missing Subject is skipped.
        assert!(parse_loadouts(&json!({ "Loadouts": [ { "Loadout": { "Items": {} } } ] })).is_empty());
    }
}
