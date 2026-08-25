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
/// Not used by the peak-rank act label (that resolves the peak's own season); kept as the
/// vRY-parity hook for a future "previous act rank" column.
#[allow(dead_code)]
pub fn previous_season_id(seasons: &[Season]) -> Option<String> {
    let current = seasons.iter().find(|s| s.season_type == "act" && s.is_active)?;
    seasons
        .iter()
        .find(|s| s.season_type == "act" && !s.start_time.is_empty() && s.end_time == current.start_time)
        .map(|s| s.id.clone())
}

/// Short label for the act a peak rank was achieved in ("E6: A3", "V26: A1").
///
/// Which episode/act an id belongs to is derived exactly as vRY does
/// (`Content.get_act_episode_from_act_id` + the has-letter test `main.py` applies to the
/// episode): the act's own number, prefixed by the episode that owns it — `E{ep}: A{act}`,
/// or `{ep}: A{act}` when the episode identifier already carries a letter (the V-era "V26"
/// naming). Only the display format differs from vRY's `(e6a3)`. `None` when the id is not
/// an act in the season list, or either half cannot be parsed.
pub fn act_label(seasons: &[Season], act_id: &str) -> Option<String> {
    let mut act: Option<String> = None;
    let mut episode: Option<String> = None;
    // vRY seeds the "episode so far" with the first season, then tracks every episode it
    // passes; the act's own episode is the last one seen before the *next* episode entry.
    let mut last_episode = seasons.first();
    let mut act_found = false;

    for season in seasons {
        if season.id.eq_ignore_ascii_case(act_id) {
            act = parse_season_number(&season.name);
            act_found = true;
        }
        if season.season_type == "episode" {
            if act_found {
                episode = last_episode.and_then(|e| parse_season_number(&e.name));
                break;
            }
            last_episode = Some(season);
        }
    }

    // The newest act has no episode entry after it, so the loop above never breaks on it.
    // vRY leaves the episode unset there (printing a literal "None"); the owning episode is
    // simply the last one passed.
    if act_found && episode.is_none() {
        episode = last_episode.and_then(|e| parse_season_number(&e.name));
    }

    let (episode, act) = (episode?, act?);
    let label = if episode.chars().any(|c| c.is_alphabetic()) {
        format!("{episode}: A{act}")
    } else {
        format!("E{episode}: A{act}")
    };
    Some(label.to_uppercase())
}

/// vRY `parse_season_number`: the trailing token of a season name as its number — Arabic or
/// Roman, whichever that season type uses — or the token itself when it mixes letters and
/// digits (the V-era naming, which is already the label Riot shows).
fn parse_season_number(name: &str) -> Option<String> {
    let token = name.split_whitespace().next_back()?;
    let has_letter = token.chars().any(|c| c.is_alphabetic());
    let has_digit = token.chars().any(|c| c.is_ascii_digit());
    if has_letter && has_digit {
        return Some(token.to_string());
    }
    // Episodes are numbered with digits, acts with Roman numerals; each falls back to the
    // other, exactly as vRY does.
    if name.starts_with("EPISODE") {
        token.parse::<u32>().ok().or_else(|| roman_to_int(token))
    } else if name.starts_with("ACT") {
        roman_to_int(token).or_else(|| token.parse::<u32>().ok())
    } else {
        None
    }
    .map(|n| n.to_string())
}

/// Roman numeral -> integer (subtractive notation). `None` on any non-Roman character.
fn roman_to_int(roman: &str) -> Option<u32> {
    let mut total: i64 = 0;
    let mut prev: i64 = 0;
    for c in roman.chars().rev() {
        let value = match c.to_ascii_uppercase() {
            'I' => 1,
            'V' => 5,
            'X' => 10,
            'L' => 50,
            'C' => 100,
            _ => return None,
        };
        if value < prev {
            total -= value;
        } else {
            total += value;
        }
        prev = value;
    }
    u32::try_from(total).ok()
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

    /// Shaped like the real content-service list: each episode entry followed by its acts,
    /// including the V-era naming where the "episode" carries the letter+digit id.
    fn season_history() -> Vec<Season> {
        parse_seasons(&json!({ "Seasons": [
            { "ID": "e6", "Name": "EPISODE 6", "Type": "episode" },
            { "ID": "e6a1", "Name": "ACT I", "Type": "act" },
            { "ID": "e6a3", "Name": "ACT III", "Type": "act" },
            { "ID": "v26", "Name": "V26", "Type": "episode" },
            { "ID": "v26a1", "Name": "ACT I", "Type": "act" },
            { "ID": "v26a2", "Name": "ACT II", "Type": "act", "IsActive": true }
        ]}))
    }

    #[test]
    fn act_label_uses_episode_era_prefix() {
        let seasons = season_history();
        assert_eq!(act_label(&seasons, "e6a3").as_deref(), Some("E6: A3"));
        assert_eq!(act_label(&seasons, "e6a1").as_deref(), Some("E6: A1"));
    }

    #[test]
    fn act_label_keeps_v_era_identifier_as_the_prefix() {
        // The V-era "episode" name is already a letter+digit id, so it is used verbatim
        // instead of being prefixed with "E".
        assert_eq!(act_label(&season_history(), "v26a1").as_deref(), Some("V26: A1"));
    }

    #[test]
    fn act_label_resolves_the_newest_act_with_no_episode_after_it() {
        assert_eq!(act_label(&season_history(), "v26a2").as_deref(), Some("V26: A2"));
    }

    #[test]
    fn act_label_matches_id_case_insensitively() {
        assert_eq!(act_label(&season_history(), "E6A3").as_deref(), Some("E6: A3"));
    }

    #[test]
    fn act_label_is_none_for_unknown_id() {
        assert_eq!(act_label(&season_history(), "not-a-season"), None);
        assert_eq!(act_label(&[], "e6a3"), None);
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
