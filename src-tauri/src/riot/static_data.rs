//! valorant-api.com static data: parsing + lookup + a version-keyed on-disk cache.
//! Parsing/lookup functions are pure and fixture-testable; fetching/caching is IO.

use crate::riot::error::Result;
use crate::riot::rank::tier_name;
use crate::riot::types::{AgentInfo, MapInfo, RankInfo};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Base URL for all static-data fetches.
pub const VALORANT_API: &str = "https://valorant-api.com/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MapStatic {
    /// e.g. "/Game/Maps/Ascent/Ascent" — matched against coregame MapID (case-insensitive).
    map_url: String,
    display_name: String,
    splash: Option<String>,
    list_view_icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentStatic {
    display_name: String,
    display_icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TierStatic {
    small_icon: Option<String>,
    large_icon: Option<String>,
}

/// Cached, parsed static data for one game version.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StaticData {
    pub version: String,
    /// lowercased agent uuid -> agent.
    agents: HashMap<String, AgentStatic>,
    maps: Vec<MapStatic>,
    /// CompetitiveTier number -> icons.
    tiers: HashMap<u8, TierStatic>,
}

impl StaticData {
    /// Resolve an agent uuid (any case) to display-ready info. None -> None.
    pub fn agent(&self, character_id: Option<&str>) -> Option<AgentInfo> {
        let id = character_id?.to_lowercase();
        let a = self.agents.get(&id);
        Some(AgentInfo {
            id: id.clone(),
            name: a.map(|a| a.display_name.clone()).unwrap_or_default(),
            icon_url: a.and_then(|a| a.display_icon.clone()),
        })
    }

    /// Resolve a coregame/pregame MapID path to display-ready info (case-insensitive).
    pub fn map(&self, map_id: Option<&str>) -> Option<MapInfo> {
        let id = map_id?;
        let needle = id.to_lowercase();
        let found = self.maps.iter().find(|m| m.map_url.to_lowercase() == needle);
        Some(MapInfo {
            id: id.to_string(),
            name: found.map(|m| m.display_name.clone()).unwrap_or_default(),
            splash_url: found.and_then(|m| m.splash.clone()),
            list_view_url: found.and_then(|m| m.list_view_icon.clone()),
        })
    }

    /// Resolve a CompetitiveTier number to a display rank (name + icon). Tier 0 -> Unranked
    /// with no icon.
    pub fn rank(&self, tier: u8) -> RankInfo {
        if tier == 0 {
            return RankInfo::unranked();
        }
        RankInfo {
            tier,
            name: tier_name(tier).to_string(),
            icon_url: self
                .tiers
                .get(&tier)
                .and_then(|t| t.large_icon.clone().or_else(|| t.small_icon.clone())),
        }
    }
}

// --- pure parsers -----------------------------------------------------------

/// Parse `data.riotClientVersion` from the `/version` payload.
pub fn parse_version(value: &Value) -> Option<String> {
    value
        .get("data")?
        .get("riotClientVersion")?
        .as_str()
        .map(String::from)
}

/// Parse the `/agents` payload into lowercased-uuid -> agent.
fn parse_agents(value: &Value) -> HashMap<String, AgentStatic> {
    let mut out = HashMap::new();
    let Some(arr) = value.get("data").and_then(|d| d.as_array()) else {
        return out;
    };
    for a in arr {
        let Some(uuid) = a.get("uuid").and_then(|v| v.as_str()) else { continue };
        // Skip the non-playable duplicate agent entries when the flag is present.
        if a.get("isPlayableCharacter").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        out.insert(
            uuid.to_lowercase(),
            AgentStatic {
                display_name: a
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                display_icon: a.get("displayIcon").and_then(|v| v.as_str()).map(String::from),
            },
        );
    }
    out
}

/// Parse the `/maps` payload into a list of maps.
fn parse_maps(value: &Value) -> Vec<MapStatic> {
    let Some(arr) = value.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|m| {
            Some(MapStatic {
                map_url: m.get("mapUrl").and_then(|v| v.as_str())?.to_string(),
                display_name: m
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                splash: m.get("splash").and_then(|v| v.as_str()).map(String::from),
                list_view_icon: m.get("listViewIcon").and_then(|v| v.as_str()).map(String::from),
            })
        })
        .filter(|m| !m.map_url.is_empty())
        .collect()
}

/// Parse the `/competitivetiers` payload. Uses the LAST entry of the top-level `data`
/// array (episode tier tables are appended chronologically -> newest is current).
fn parse_competitive_tiers(value: &Value) -> HashMap<u8, TierStatic> {
    let mut out = HashMap::new();
    let Some(table) = value.get("data").and_then(|d| d.as_array()).and_then(|a| a.last()) else {
        return out;
    };
    let Some(tiers) = table.get("tiers").and_then(|t| t.as_array()) else {
        return out;
    };
    for t in tiers {
        let Some(tier) = t.get("tier").and_then(|v| v.as_u64()) else { continue };
        out.insert(
            tier as u8,
            TierStatic {
                small_icon: t.get("smallIcon").and_then(|v| v.as_str()).map(String::from),
                large_icon: t.get("largeIcon").and_then(|v| v.as_str()).map(String::from),
            },
        );
    }
    out
}

/// Build `StaticData` from the four already-fetched payloads.
pub fn build(version: String, agents: &Value, maps: &Value, tiers: &Value) -> StaticData {
    StaticData {
        version,
        agents: parse_agents(agents),
        maps: parse_maps(maps),
        tiers: parse_competitive_tiers(tiers),
    }
}

// --- IO: fetch + disk cache -------------------------------------------------

/// Directory for the version-keyed static-data cache (`%LOCALAPPDATA%\...\static-cache`).
fn cache_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let mut p = std::path::PathBuf::from(base);
    p.push("valorant-lightweight-tracker");
    p.push("static-cache");
    Some(p)
}

fn cache_file(version: &str) -> Option<std::path::PathBuf> {
    let mut p = cache_dir()?;
    // Sanitize version for use as a filename.
    let safe: String = version
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
        .collect();
    p.push(format!("static-{safe}.json"));
    Some(p)
}

/// Load cached static data for `version` from disk, if present and valid.
pub fn load_cache(version: &str) -> Option<StaticData> {
    let path = cache_file(version)?;
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Persist static data to the version-keyed cache file.
pub fn save_cache(data: &StaticData) -> Result<()> {
    if let Some(dir) = cache_dir() {
        std::fs::create_dir_all(&dir)?;
    }
    if let Some(path) = cache_file(&data.version) {
        let bytes = serde_json::to_vec(data)?;
        std::fs::write(path, bytes)?;
    }
    Ok(())
}

/// Fetch + parse all static data from valorant-api.com, using the disk cache when the
/// version already matches. `client` is an ordinary reqwest client (public host, valid TLS).
pub async fn fetch(client: &reqwest::Client) -> Result<StaticData> {
    let version_json: Value = client.get(format!("{VALORANT_API}/version")).send().await?.json().await?;
    let version = parse_version(&version_json).unwrap_or_default();

    if !version.is_empty() {
        if let Some(cached) = load_cache(&version) {
            return Ok(cached);
        }
    }

    let agents: Value =
        client.get(format!("{VALORANT_API}/agents?isPlayableCharacter=true")).send().await?.json().await?;
    let maps: Value = client.get(format!("{VALORANT_API}/maps")).send().await?.json().await?;
    let tiers: Value =
        client.get(format!("{VALORANT_API}/competitivetiers")).send().await?.json().await?;

    let data = build(version, &agents, &maps, &tiers);
    let _ = save_cache(&data); // best-effort cache write
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_version() {
        let v = json!({ "data": { "riotClientVersion": "release-13.04-shipping-18-5304478" } });
        assert_eq!(parse_version(&v).as_deref(), Some("release-13.04-shipping-18-5304478"));
    }

    #[test]
    fn resolves_agent_case_insensitively() {
        let agents = json!({ "data": [
            { "uuid": "ADD6443A-41BD-E414-F6AD-E58D267F4E95", "displayName": "Reyna",
              "displayIcon": "https://x/reyna.png", "isPlayableCharacter": true }
        ]});
        let data = build("v".into(), &agents, &json!({}), &json!({}));
        let a = data.agent(Some("add6443a-41bd-e414-f6ad-e58d267f4e95")).unwrap();
        assert_eq!(a.name, "Reyna");
        assert_eq!(a.icon_url.as_deref(), Some("https://x/reyna.png"));
        // unknown agent still returns id with empty name.
        let unknown = data.agent(Some("deadbeef")).unwrap();
        assert_eq!(unknown.name, "");
        assert!(unknown.icon_url.is_none());
        assert!(data.agent(None).is_none());
    }

    #[test]
    fn skips_non_playable_agents() {
        let agents = json!({ "data": [
            { "uuid": "dup", "displayName": "Sova(NPE)", "isPlayableCharacter": false }
        ]});
        let data = build("v".into(), &agents, &json!({}), &json!({}));
        // still returns an id-only info (not in map) but the entry was skipped.
        assert_eq!(data.agent(Some("dup")).unwrap().name, "");
    }

    #[test]
    fn resolves_map_by_url_case_insensitive() {
        let maps = json!({ "data": [
            { "uuid": "m1", "mapUrl": "/Game/Maps/Ascent/Ascent", "displayName": "Ascent",
              "splash": "https://x/splash.png", "listViewIcon": "https://x/list.png" }
        ]});
        let data = build("v".into(), &json!({}), &maps, &json!({}));
        let m = data.map(Some("/game/maps/ascent/ascent")).unwrap();
        assert_eq!(m.name, "Ascent");
        assert_eq!(m.splash_url.as_deref(), Some("https://x/splash.png"));
    }

    #[test]
    fn picks_last_competitive_tier_table() {
        let tiers = json!({ "data": [
            { "tiers": [ { "tier": 27, "tierName": "OLD", "largeIcon": "old.png" } ] },
            { "tiers": [ { "tier": 27, "tierName": "RADIANT", "smallIcon": "s.png", "largeIcon": "new.png" } ] }
        ]});
        let data = build("v".into(), &json!({}), &json!({}), &tiers);
        let r = data.rank(27);
        assert_eq!(r.name, "Radiant");
        assert_eq!(r.icon_url.as_deref(), Some("new.png")); // from the LAST table
    }

    #[test]
    fn tier_zero_is_unranked_without_icon() {
        let data = StaticData::default();
        let r = data.rank(0);
        assert_eq!(r.tier, 0);
        assert_eq!(r.name, "Unranked");
        assert!(r.icon_url.is_none());
    }
}
