//! Presence decoding. Riot has been actively switching between two private-presence
//! shapes, so every field is read defensively from BOTH the nested and flat location.
//! All functions here are pure and testable from JSON fixtures.

use crate::riot::error::{Error, Result};
use crate::riot::types::SessionLoopState;
use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

/// A raw presence entry from `/chat/v4/presences` or the websocket push.
#[derive(Debug, Clone, Deserialize)]
pub struct RawPresence {
    pub puuid: String,
    #[serde(default)]
    pub product: Option<String>,
    /// base64 of a JSON blob (Valorant private presence). Empty/absent = not initialized.
    #[serde(default)]
    pub private: Option<String>,
    /// Present on League presence entries — a signal to skip.
    #[serde(default, rename = "championId")]
    pub champion_id: Option<Value>,
}

impl RawPresence {
    /// True if this entry is a Valorant presence we should decode. Skip League entries
    /// (product == "league_of_legends" or a championId present) — pitfall §10.
    pub fn is_valorant(&self) -> bool {
        if self.champion_id.is_some() {
            return false;
        }
        match self.product.as_deref() {
            Some("league_of_legends") => false,
            Some("valorant") => true,
            // Unknown/absent product: treat as Valorant only if it carries a private blob.
            _ => self.private.as_deref().map(|p| !p.is_empty()).unwrap_or(false),
        }
    }
}

/// Fields extracted from a decoded private presence.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PresenceInfo {
    pub session_state: Option<SessionLoopState>,
    pub party_state: Option<String>,
    pub provisioning_flow: Option<String>,
    pub queue_id: Option<String>,
    pub account_level: Option<u32>,
    pub party_id: Option<String>,
    pub party_size: Option<u32>,
    /// Client version read from own presence (`partyPresenceData.partyClientVersion`, or the
    /// flat `partyClientVersion`). Once connected this is authoritative over valorant-api.com,
    /// which can lag the real client on patch days (spec Live verification).
    pub client_version: Option<String>,
}

impl PresenceInfo {
    /// True when this presence describes a custom game (relabel mode as "Custom").
    /// Detected via provisioningFlow == "CustomGame" OR partyState == "CUSTOM_GAME_SETUP".
    pub fn is_custom_game(&self) -> bool {
        self.provisioning_flow.as_deref() == Some("CustomGame")
            || self.party_state.as_deref() == Some("CUSTOM_GAME_SETUP")
    }
}

/// Base64-decode + JSON-parse the `private` blob. Empty string => `NotReady` (presence
/// not yet initialized — poll again).
pub fn decode_private(private_b64: &str) -> Result<Value> {
    if private_b64.is_empty() {
        return Err(Error::NotReady);
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(private_b64)
        .map_err(|e| Error::Decode(format!("base64: {e}")))?;
    let value: Value = serde_json::from_slice(&bytes)?;
    Ok(value)
}

/// Read `key` first from `decoded[nested_parent][key]`, then fall back to `decoded[key]`.
fn dual<'a>(decoded: &'a Value, nested_parent: &str, key: &str) -> Option<&'a Value> {
    decoded
        .get(nested_parent)
        .and_then(|p| p.get(key))
        .or_else(|| decoded.get(key))
}

/// Extract the useful fields from a decoded private presence, handling both shapes.
pub fn extract_info(decoded: &Value) -> PresenceInfo {
    let session_state = dual(decoded, "matchPresenceData", "sessionLoopState")
        .and_then(|v| v.as_str())
        .and_then(SessionLoopState::from_str);

    let party_state = dual(decoded, "partyPresenceData", "partyState")
        .and_then(|v| v.as_str())
        .map(String::from);

    // provisioningFlow and queueId are documented as top-level on the decoded object,
    // but read them dual-path too for robustness.
    let provisioning_flow = dual(decoded, "matchPresenceData", "provisioningFlow")
        .and_then(|v| v.as_str())
        .map(String::from);

    let queue_id = dual(decoded, "matchPresenceData", "queueId")
        .and_then(|v| v.as_str())
        .map(String::from);

    let account_level = dual(decoded, "playerPresenceData", "accountLevel")
        .and_then(json_u32);

    let party_id = dual(decoded, "partyPresenceData", "partyId")
        .and_then(|v| v.as_str())
        .map(String::from);

    let party_size = dual(decoded, "partyPresenceData", "partySize").and_then(json_u32);

    let client_version = dual(decoded, "partyPresenceData", "partyClientVersion")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    PresenceInfo {
        session_state,
        party_state,
        provisioning_flow,
        queue_id,
        account_level,
        party_id,
        party_size,
        client_version,
    }
}

/// Accept a numeric field whether it arrives as a JSON number or a numeric string.
fn json_u32(v: &Value) -> Option<u32> {
    if let Some(n) = v.as_u64() {
        return Some(n as u32);
    }
    v.as_str().and_then(|s| s.parse::<u32>().ok())
}

/// Decode + extract in one step from a raw presence entry.
pub fn info_for(raw: &RawPresence) -> Result<PresenceInfo> {
    let private = raw.private.as_deref().unwrap_or("");
    let decoded = decode_private(private)?;
    Ok(extract_info(&decoded))
}

/// Build puuid -> partyId grouping across every online presence, keeping only real
/// parties (partySize > 1). LoL entries are skipped; undecodable entries are ignored.
pub fn party_grouping(presences: &[RawPresence]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for p in presences {
        if !p.is_valorant() {
            continue;
        }
        if let Ok(info) = info_for(p) {
            if let (Some(id), Some(size)) = (info.party_id, info.party_size) {
                if size > 1 {
                    out.insert(p.puuid.clone(), id);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn encode(v: &Value) -> String {
        base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(v).unwrap())
    }

    #[test]
    fn empty_private_is_not_ready() {
        assert!(matches!(decode_private(""), Err(Error::NotReady)));
    }

    #[test]
    fn decodes_nested_shape() {
        let decoded = json!({
            "matchPresenceData": { "sessionLoopState": "INGAME", "queueId": "competitive" },
            "partyPresenceData": { "partyState": "DEFAULT", "partyId": "party-1", "partySize": 3 },
            "playerPresenceData": { "accountLevel": 275 }
        });
        let info = extract_info(&decoded);
        assert_eq!(info.session_state, Some(SessionLoopState::Ingame));
        assert_eq!(info.queue_id.as_deref(), Some("competitive"));
        assert_eq!(info.account_level, Some(275));
        assert_eq!(info.party_id.as_deref(), Some("party-1"));
        assert_eq!(info.party_size, Some(3));
    }

    #[test]
    fn decodes_flat_shape() {
        let decoded = json!({
            "sessionLoopState": "PREGAME",
            "queueId": "unrated",
            "partyState": "DEFAULT",
            "partyId": "party-2",
            "partySize": 1,
            "accountLevel": 42
        });
        let info = extract_info(&decoded);
        assert_eq!(info.session_state, Some(SessionLoopState::Pregame));
        assert_eq!(info.queue_id.as_deref(), Some("unrated"));
        assert_eq!(info.account_level, Some(42));
        assert_eq!(info.party_size, Some(1));
    }

    #[test]
    fn account_level_as_string_is_accepted() {
        let decoded = json!({ "sessionLoopState": "MENUS", "accountLevel": "310" });
        let info = extract_info(&decoded);
        assert_eq!(info.account_level, Some(310));
    }

    #[test]
    fn detects_custom_game_via_provisioning_flow() {
        let decoded = json!({ "sessionLoopState": "INGAME", "provisioningFlow": "CustomGame" });
        assert!(extract_info(&decoded).is_custom_game());
    }

    #[test]
    fn detects_custom_game_via_party_state() {
        let decoded = json!({ "sessionLoopState": "MENUS", "partyState": "CUSTOM_GAME_SETUP" });
        assert!(extract_info(&decoded).is_custom_game());
    }

    #[test]
    fn reads_client_version_nested_and_flat() {
        let nested = json!({
            "sessionLoopState": "MENUS",
            "partyPresenceData": { "partyClientVersion": "release-13.04-shipping-20-5340415" }
        });
        assert_eq!(
            extract_info(&nested).client_version.as_deref(),
            Some("release-13.04-shipping-20-5340415")
        );
        let flat = json!({
            "sessionLoopState": "MENUS",
            "partyClientVersion": "release-13.04-shipping-20-5340415"
        });
        assert_eq!(
            extract_info(&flat).client_version.as_deref(),
            Some("release-13.04-shipping-20-5340415")
        );
        // Absent -> None (falls back to the valorant-api bootstrap version).
        assert_eq!(extract_info(&json!({ "sessionLoopState": "MENUS" })).client_version, None);
    }

    #[test]
    fn info_for_empty_private_is_not_ready() {
        // Own presence with an empty `private` blob must surface NotReady (poll again),
        // not decode to a default (menus) info (C10).
        let raw = RawPresence {
            puuid: "me".into(),
            product: Some("valorant".into()),
            private: Some(String::new()),
            champion_id: None,
        };
        assert!(matches!(info_for(&raw), Err(Error::NotReady)));
    }

    #[test]
    fn round_trips_base64_private() {
        let decoded = json!({ "sessionLoopState": "INGAME" });
        let info = info_for(&RawPresence {
            puuid: "p".into(),
            product: Some("valorant".into()),
            private: Some(encode(&decoded)),
            champion_id: None,
        })
        .unwrap();
        assert_eq!(info.session_state, Some(SessionLoopState::Ingame));
    }

    #[test]
    fn skips_league_presence() {
        let lol = RawPresence {
            puuid: "p".into(),
            product: Some("league_of_legends".into()),
            private: Some("x".into()),
            champion_id: Some(json!(157)),
        };
        assert!(!lol.is_valorant());
    }

    #[test]
    fn party_grouping_keeps_only_real_parties() {
        let solo = json!({ "sessionLoopState": "MENUS", "partyId": "solo", "partySize": 1 });
        let duo_a = json!({ "sessionLoopState": "MENUS", "partyId": "duo", "partySize": 2 });
        let duo_b = json!({ "sessionLoopState": "MENUS", "partyId": "duo", "partySize": 2 });
        let presences = vec![
            RawPresence { puuid: "s".into(), product: Some("valorant".into()), private: Some(encode(&solo)), champion_id: None },
            RawPresence { puuid: "a".into(), product: Some("valorant".into()), private: Some(encode(&duo_a)), champion_id: None },
            RawPresence { puuid: "b".into(), product: Some("valorant".into()), private: Some(encode(&duo_b)), champion_id: None },
        ];
        let grouping = party_grouping(&presences);
        assert!(!grouping.contains_key("s"));
        assert_eq!(grouping.get("a"), Some(&"duo".to_string()));
        assert_eq!(grouping.get("b"), Some(&"duo".to_string()));
    }
}
