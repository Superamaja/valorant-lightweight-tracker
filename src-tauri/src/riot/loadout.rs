//! Phase 2 coregame loadout parsing: extract each player's equipped Vandal/Phantom skin
//! uuid from `/core-game/v1/matches/{id}/loadouts`. Pure and fixture-testable; skin uuid ->
//! name/icon resolution happens later against `StaticData`. See spec Live verification 2.

use crate::riot::constants::{
    CHROMA_SOCKET_ID, PHANTOM_WEAPON_ID, SKIN_SOCKET_ID, VANDAL_WEAPON_ID,
};
use serde_json::Value;
use std::collections::HashMap;

/// One weapon's equipped skin and, when the player picked one, its chroma (colourway).
/// Either half is `None` when the loadout carries no such socket.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EquippedSkin {
    pub skin: Option<String>,
    pub chroma: Option<String>,
}

/// The equipped skins for the two rifles we surface. Empty for a weapon the player has no
/// loadout entry for (shouldn't happen for base rifles, but handled defensively).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayerSkinIds {
    pub vandal: EquippedSkin,
    pub phantom: EquippedSkin,
}

/// Read a socket's `Item.ID` for `weapon_id` from a loadout's `Items` map:
/// `Items[weapon_id].Sockets[socket_id].Item.ID`.
fn socket_item_id(items: &Value, weapon_id: &str, socket_id: &str) -> Option<String> {
    items
        .get(weapon_id)?
        .get("Sockets")?
        .get(socket_id)?
        .get("Item")?
        .get("ID")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// The skin + chroma pair equipped on `weapon_id`.
fn equipped_skin_for(items: &Value, weapon_id: &str) -> EquippedSkin {
    EquippedSkin {
        skin: socket_item_id(items, weapon_id, SKIN_SOCKET_ID),
        chroma: socket_item_id(items, weapon_id, CHROMA_SOCKET_ID),
    }
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
                vandal: equipped_skin_for(items, VANDAL_WEAPON_ID),
                phantom: equipped_skin_for(items, PHANTOM_WEAPON_ID),
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
                    CHROMA_SOCKET_ID: { "ID": CHROMA_SOCKET_ID, "Item": { "ID": "chroma-neptune-2" } },
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
        assert_eq!(a.vandal.skin.as_deref(), Some("skin-neptune-vandal"));
        assert_eq!(a.vandal.chroma.as_deref(), Some("chroma-neptune-2"));
        assert_eq!(a.phantom.skin.as_deref(), Some("skin-reaver-phantom"));
        // No chroma socket on the Phantom -> no chroma.
        assert_eq!(a.phantom.chroma, None);
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
        assert_eq!(p.vandal.skin.as_deref(), Some("van-skin"));
        assert_eq!(p.phantom, EquippedSkin::default());
    }

    #[test]
    fn empty_or_malformed_is_empty_map() {
        assert!(parse_loadouts(&json!({})).is_empty());
        assert!(parse_loadouts(&json!({ "Loadouts": [] })).is_empty());
        // Entry missing Subject is skipped.
        assert!(parse_loadouts(&json!({ "Loadouts": [ { "Loadout": { "Items": {} } } ] })).is_empty());
    }
}
