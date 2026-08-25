//! valorant-api.com static data: parsing + lookup + a version-keyed on-disk cache.
//! Parsing/lookup functions are pure and fixture-testable; fetching/caching is IO.

use crate::riot::constants::{PHANTOM_WEAPON_ID, VANDAL_WEAPON_ID};
use crate::riot::error::Result;
use crate::riot::rank::tier_name;
use crate::riot::types::{AgentInfo, MapInfo, RankInfo, SkinInfo};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SkinStatic {
    display_name: String,
    display_icon: Option<String>,
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
    /// lowercased weapon-skin uuid -> name + icon (phase 2: Vandal/Phantom skins).
    #[serde(default)]
    skins: HashMap<String, SkinStatic>,
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

    /// Resolve a weapon-skin uuid (any case) to display-ready info. `None` skin id -> None
    /// (weapon not equipped / pregame). An unknown uuid still returns an entry with an empty
    /// name so the UI can decide how to render it.
    pub fn skin(&self, skin_id: Option<&str>) -> Option<SkinInfo> {
        let id = skin_id?.to_lowercase();
        let s = self.skins.get(&id);
        Some(SkinInfo {
            name: s.map(|s| s.display_name.clone()).unwrap_or_default(),
            icon_url: s.and_then(|s| s.display_icon.clone()),
        })
    }

    /// Resolve a CompetitiveTier number to a display rank (name + icon). Tier 0 goes through
    /// the same table lookup as every other tier — the competitivetiers table carries a real
    /// Unranked icon — and falls back to `RankInfo::unranked()` shape (name "Unranked", no
    /// icon) when the table has no tier-0 entry.
    pub fn rank(&self, tier: u8) -> RankInfo {
        let icon_url = self
            .tiers
            .get(&tier)
            .and_then(|t| t.large_icon.clone().or_else(|| t.small_icon.clone()));
        if tier == 0 && icon_url.is_none() {
            return RankInfo::unranked();
        }
        RankInfo { tier, name: tier_name(tier).to_string(), icon_url }
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

/// Parse the `/weapons` payload, keeping ONLY the Vandal + Phantom skins (matched by the
/// parent weapon uuid) — the two rifles the table surfaces. Filtering by weapon here instead
/// of storing all ~5k skins from `/weapons/skins` keeps the version-keyed disk cache tiny.
/// Matching on the parent weapon (rather than a skin `assetPath` heuristic) is the robust
/// route: the two weapons' `skins` arrays are exactly the skins we need. Some skins have a
/// null top-level `displayIcon` (default/random skins); fall back to the first level's icon.
fn parse_weapon_skins(value: &Value) -> HashMap<String, SkinStatic> {
    let mut out = HashMap::new();
    let Some(arr) = value.get("data").and_then(|d| d.as_array()) else {
        return out;
    };
    for weapon in arr {
        let Some(weapon_uuid) = weapon.get("uuid").and_then(|v| v.as_str()) else { continue };
        let weapon_uuid = weapon_uuid.to_lowercase();
        if weapon_uuid != VANDAL_WEAPON_ID && weapon_uuid != PHANTOM_WEAPON_ID {
            continue;
        }
        let Some(skins) = weapon.get("skins").and_then(|s| s.as_array()) else { continue };
        for s in skins {
            let Some(uuid) = s.get("uuid").and_then(|v| v.as_str()) else { continue };
            let display_icon = s
                .get("displayIcon")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    s.get("levels")
                        .and_then(|l| l.as_array())
                        .and_then(|l| l.first())
                        .and_then(|lvl| lvl.get("displayIcon"))
                        .and_then(|v| v.as_str())
                })
                .map(String::from);
            out.insert(
                uuid.to_lowercase(),
                SkinStatic {
                    display_name: s
                        .get("displayName")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    display_icon,
                },
            );
        }
    }
    out
}

/// Build `StaticData` from the already-fetched payloads. `weapons` is the `/weapons`
/// payload (the Vandal + Phantom skins are extracted from it).
pub fn build(
    version: String,
    agents: &Value,
    maps: &Value,
    tiers: &Value,
    weapons: &Value,
) -> StaticData {
    StaticData {
        version,
        agents: parse_agents(agents),
        maps: parse_maps(maps),
        tiers: parse_competitive_tiers(tiers),
        skins: parse_weapon_skins(weapons),
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
    // `/weapons` (not `/weapons/skins`) so we can filter to just the Vandal + Phantom skins
    // by their parent weapon uuid — far smaller than the ~5k-entry full skin list.
    let weapons: Value =
        client.get(format!("{VALORANT_API}/weapons")).send().await?.json().await?;

    let data = build(version, &agents, &maps, &tiers, &weapons);
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
        let data = build("v".into(), &agents, &json!({}), &json!({}), &json!({}));
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
        let data = build("v".into(), &agents, &json!({}), &json!({}), &json!({}));
        // still returns an id-only info (not in map) but the entry was skipped.
        assert_eq!(data.agent(Some("dup")).unwrap().name, "");
    }

    #[test]
    fn resolves_map_by_url_case_insensitive() {
        let maps = json!({ "data": [
            { "uuid": "m1", "mapUrl": "/Game/Maps/Ascent/Ascent", "displayName": "Ascent",
              "splash": "https://x/splash.png", "listViewIcon": "https://x/list.png" }
        ]});
        let data = build("v".into(), &json!({}), &maps, &json!({}), &json!({}));
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
        let data = build("v".into(), &json!({}), &json!({}), &tiers, &json!({}));
        let r = data.rank(27);
        assert_eq!(r.name, "Radiant");
        assert_eq!(r.icon_url.as_deref(), Some("new.png")); // from the LAST table
    }

    #[test]
    fn resolves_skin_case_insensitively_with_icon_fallback() {
        // `/weapons` shape: each weapon carries its own `skins` array. Only the Vandal +
        // Phantom weapons' skins are kept; every other weapon's skins are filtered out.
        let weapons = json!({ "data": [
            { "uuid": VANDAL_WEAPON_ID, "displayName": "Vandal", "skins": [
                { "uuid": "DB91451C-4309-2C8C-EDED-BF842D844E52", "displayName": "Neptune Vandal",
                  "displayIcon": "https://x/neptune.png" },
                // default skin: null top-level displayIcon, icon in levels[0]
                { "uuid": "fallback-skin", "displayName": "Standard Vandal", "displayIcon": null,
                  "levels": [ { "displayIcon": "https://x/standard.png" } ] }
            ]},
            { "uuid": PHANTOM_WEAPON_ID, "displayName": "Phantom", "skins": [
                { "uuid": "reaver-phantom", "displayName": "Reaver Phantom",
                  "displayIcon": "https://x/reaver.png" }
            ]},
            // a non-rifle weapon whose skins must be filtered out.
            { "uuid": "some-sheriff-weapon", "displayName": "Sheriff", "skins": [
                { "uuid": "ignored-skin", "displayName": "Ignore Me", "displayIcon": "https://x/i.png" }
            ]}
        ]});
        let data = build("v".into(), &json!({}), &json!({}), &json!({}), &weapons);
        let van = data.skin(Some("db91451c-4309-2c8c-eded-bf842d844e52")).unwrap();
        assert_eq!(van.name, "Neptune Vandal");
        assert_eq!(van.icon_url.as_deref(), Some("https://x/neptune.png"));
        // level fallback for a null top-level icon.
        let fb = data.skin(Some("fallback-skin")).unwrap();
        assert_eq!(fb.icon_url.as_deref(), Some("https://x/standard.png"));
        // phantom skin kept too.
        assert_eq!(data.skin(Some("reaver-phantom")).unwrap().name, "Reaver Phantom");
        // a non-Vandal/Phantom weapon's skin is filtered out -> resolves to an empty entry.
        assert_eq!(data.skin(Some("ignored-skin")).unwrap().name, "");
        // unknown uuid -> empty name, no icon.
        let unknown = data.skin(Some("nope")).unwrap();
        assert_eq!(unknown.name, "");
        assert!(unknown.icon_url.is_none());
        // None -> None.
        assert!(data.skin(None).is_none());
    }

    #[test]
    fn tier_zero_uses_the_tables_unranked_icon() {
        let tiers = json!({ "data": [
            { "tiers": [ { "tier": 0, "tierName": "UNRANKED", "smallIcon": null,
                           "largeIcon": "https://x/unranked.png" } ] }
        ]});
        let data = build("v".into(), &json!({}), &json!({}), &tiers, &json!({}));
        let r = data.rank(0);
        assert_eq!(r.tier, 0);
        assert_eq!(r.name, "Unranked");
        assert_eq!(r.icon_url.as_deref(), Some("https://x/unranked.png"));
    }

    #[test]
    fn tier_zero_without_a_table_entry_is_unranked_without_icon() {
        let data = StaticData::default();
        assert_eq!(data.rank(0), RankInfo::unranked());
    }
}
