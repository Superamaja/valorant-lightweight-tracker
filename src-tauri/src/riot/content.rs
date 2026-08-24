//! Content-service season list parsing: current + previous act season ids. See spec §7.

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Season {
    #[serde(rename = "ID", default)]
    pub id: String,
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "Type", default)]
    pub season_type: String,
    #[serde(rename = "StartTime", default)]
    pub start_time: String,
    #[serde(rename = "EndTime", default)]
    pub end_time: String,
    #[serde(rename = "IsActive", default)]
    pub is_active: bool,
}

#[derive(Debug, Deserialize)]
struct ContentWire {
    #[serde(rename = "Seasons", default)]
    seasons: Vec<Season>,
}

/// Parse the raw content-service payload into its season list.
pub fn parse_seasons(value: &Value) -> Vec<Season> {
    serde_json::from_value::<ContentWire>(value.clone())
        .map(|c| c.seasons)
        .unwrap_or_default()
}

/// Current season id = the active `act` season.
pub fn current_season_id(seasons: &[Season]) -> Option<String> {
    seasons
        .iter()
        .find(|s| s.season_type == "act" && s.is_active)
        .map(|s| s.id.clone())
}

/// Previous season id = the `act` whose EndTime == current act's StartTime.
/// Kept for the peak-rank act-label feature (spec §7) which v1 does not yet surface.
#[allow(dead_code)]
pub fn previous_season_id(seasons: &[Season]) -> Option<String> {
    let current = seasons.iter().find(|s| s.season_type == "act" && s.is_active)?;
    seasons
        .iter()
        .find(|s| s.season_type == "act" && !s.start_time.is_empty() && s.end_time == current.start_time)
        .map(|s| s.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Value {
        json!({ "Seasons": [
            { "ID": "ep", "Name": "EPISODE 9", "Type": "episode", "StartTime": "t0", "EndTime": "t3", "IsActive": true },
            { "ID": "act-prev", "Name": "ACT I", "Type": "act", "StartTime": "t0", "EndTime": "t1", "IsActive": false },
            { "ID": "act-cur", "Name": "ACT II", "Type": "act", "StartTime": "t1", "EndTime": "t2", "IsActive": true }
        ]})
    }

    #[test]
    fn finds_current_act() {
        let seasons = parse_seasons(&fixture());
        assert_eq!(current_season_id(&seasons).as_deref(), Some("act-cur"));
    }

    #[test]
    fn finds_previous_act_by_end_time() {
        let seasons = parse_seasons(&fixture());
        assert_eq!(previous_season_id(&seasons).as_deref(), Some("act-prev"));
    }

    #[test]
    fn no_active_act_yields_none() {
        let seasons = parse_seasons(&json!({ "Seasons": [
            { "ID": "a", "Type": "act", "IsActive": false, "StartTime": "x", "EndTime": "y" }
        ]}));
        assert_eq!(current_season_id(&seasons), None);
        assert_eq!(previous_season_id(&seasons), None);
    }
}
