//! Phase 2 per-player stats parsing: competitiveupdates (ΔRR + last-N W/L pips + the
//! recent match ids HS% reuses) and match-details headshot accounting. All pure and
//! fixture-testable; the network fetching lives in `remote_api`, the orchestration/caching
//! in `app_state`.

use crate::riot::constants::{RECENT_MATCHES_FOR_HS, RECENT_RESULTS_COUNT};
use crate::riot::types::MatchResult;
use serde::Deserialize;
use serde_json::Value;

// --- competitiveupdates ------------------------------------------------------

/// Derived from `/mmr/v1/players/{puuid}/competitiveupdates`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RrHistory {
    /// ΔRR of the newest competitive match, or None when there are no matches.
    pub rr_change: Option<i32>,
    /// Up to `RECENT_RESULTS_COUNT` most recent results, newest first.
    pub results: Vec<MatchResult>,
    /// Up to `RECENT_MATCHES_FOR_HS` most recent match ids, newest first (for HS%).
    pub recent_match_ids: Vec<String>,
}

impl RrHistory {
    /// The newest competitive match id (Matches[0]), if any — the HS% cache key.
    pub fn newest_match_id(&self) -> Option<&str> {
        self.recent_match_ids.first().map(String::as_str)
    }
}

#[derive(Debug, Deserialize)]
struct CompetitiveUpdateEntry {
    #[serde(rename = "MatchID", default)]
    match_id: String,
    #[serde(rename = "RankedRatingEarned", default)]
    ranked_rating_earned: i32,
}

#[derive(Debug, Deserialize, Default)]
struct CompetitiveUpdatesWire {
    #[serde(rename = "Matches", default)]
    matches: Vec<CompetitiveUpdateEntry>,
}

/// One match's W/L from its `RankedRatingEarned` sign. A 0-earned match is ambiguous
/// (vRY treats sign only, and 0 could be a draw, an AFK-penalised win, or a placement) ->
/// `Unknown`. Note: `AFKPenalty` exists in the payload but is not used for the pip.
pub fn result_from_rr(earned: i32) -> MatchResult {
    match earned.cmp(&0) {
        std::cmp::Ordering::Greater => MatchResult::Win,
        std::cmp::Ordering::Less => MatchResult::Loss,
        std::cmp::Ordering::Equal => MatchResult::Unknown,
    }
}

/// Parse a competitiveupdates payload into ΔRR + last-N pips + recent match ids.
/// Never fails — a malformed payload degrades to an empty history.
pub fn parse_competitive_updates(value: Value) -> RrHistory {
    let wire: CompetitiveUpdatesWire = serde_json::from_value(value).unwrap_or_default();
    let rr_change = wire.matches.first().map(|m| m.ranked_rating_earned);
    let results = wire
        .matches
        .iter()
        .take(RECENT_RESULTS_COUNT)
        .map(|m| result_from_rr(m.ranked_rating_earned))
        .collect();
    let recent_match_ids = wire
        .matches
        .iter()
        .filter(|m| !m.match_id.is_empty())
        .take(RECENT_MATCHES_FOR_HS)
        .map(|m| m.match_id.clone())
        .collect();
    RrHistory { rr_change, results, recent_match_ids }
}

// --- match-details headshot accounting --------------------------------------

/// Running head/body/leg hit totals for one player across one or more matches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HitCounts {
    pub head: u64,
    pub body: u64,
    pub leg: u64,
}

impl HitCounts {
    pub fn total(&self) -> u64 {
        self.head + self.body + self.leg
    }

    /// Headshot percent = round(head / (head+body+leg) * 100). `None` when no hits recorded
    /// (vRY shows "N/a") — mirrors vRY `player_stats._process_match_data`. Uses round
    /// half-to-even to match Python's `round()` exactly (e.g. 12.5 -> 12, not 13).
    pub fn headshot_percent(&self) -> Option<u32> {
        let total = self.total();
        if total == 0 {
            return None;
        }
        Some(((self.head as f64 / total as f64) * 100.0).round_ties_even() as u32)
    }
}

/// Accumulate a single match-details payload's head/body/leg hits for `subject` into
/// `acc`, summing every damage entry across every round's `playerStats` — replicating
/// vRY `player_stats.py`. Consumes `value` (no deep clone). Missing fields default to 0.
pub fn accumulate_match_hits(acc: &mut HitCounts, value: &Value, subject: &str) {
    let Some(rounds) = value.get("roundResults").and_then(|r| r.as_array()) else {
        return;
    };
    for round in rounds {
        let Some(player_stats) = round.get("playerStats").and_then(|p| p.as_array()) else {
            continue;
        };
        for ps in player_stats {
            if ps.get("subject").and_then(|s| s.as_str()) != Some(subject) {
                continue;
            }
            let Some(damage) = ps.get("damage").and_then(|d| d.as_array()) else {
                continue;
            };
            for d in damage {
                acc.head += d.get("headshots").and_then(|v| v.as_u64()).unwrap_or(0);
                acc.body += d.get("bodyshots").and_then(|v| v.as_u64()).unwrap_or(0);
                acc.leg += d.get("legshots").and_then(|v| v.as_u64()).unwrap_or(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn result_sign_including_zero_ambiguity() {
        assert_eq!(result_from_rr(13), MatchResult::Win);
        assert_eq!(result_from_rr(-20), MatchResult::Loss);
        // 0-RR edge: ambiguous, not a loss.
        assert_eq!(result_from_rr(0), MatchResult::Unknown);
    }

    #[test]
    fn parses_delta_rr_and_last5_pips() {
        // Sanitized from the live competitiveupdates capture (RankedRatingEarned sequence
        // 13, 11, 12, -20, 9, ...). Newest ΔRR = +13; pips newest-first.
        let payload = json!({ "Matches": [
            { "MatchID": "m1", "RankedRatingEarned": 13 },
            { "MatchID": "m2", "RankedRatingEarned": 11 },
            { "MatchID": "m3", "RankedRatingEarned": 12 },
            { "MatchID": "m4", "RankedRatingEarned": -20 },
            { "MatchID": "m5", "RankedRatingEarned": 9 },
            { "MatchID": "m6", "RankedRatingEarned": 16 }
        ]});
        let h = parse_competitive_updates(payload);
        assert_eq!(h.rr_change, Some(13));
        assert_eq!(
            h.results,
            vec![
                MatchResult::Win,
                MatchResult::Win,
                MatchResult::Win,
                MatchResult::Loss,
                MatchResult::Win,
            ]
        );
        // last-5 only, even though 6 matches were present.
        assert_eq!(h.results.len(), 5);
        // recent match ids capped at RECENT_MATCHES_FOR_HS (3), newest first.
        assert_eq!(h.recent_match_ids, vec!["m1", "m2", "m3"]);
        assert_eq!(h.newest_match_id(), Some("m1"));
    }

    #[test]
    fn zero_rr_match_is_unknown_pip() {
        let payload = json!({ "Matches": [ { "MatchID": "m1", "RankedRatingEarned": 0 } ] });
        let h = parse_competitive_updates(payload);
        assert_eq!(h.rr_change, Some(0));
        assert_eq!(h.results, vec![MatchResult::Unknown]);
    }

    #[test]
    fn empty_updates_have_no_history() {
        let h = parse_competitive_updates(json!({ "Matches": [] }));
        assert_eq!(h.rr_change, None);
        assert!(h.results.is_empty());
        assert!(h.recent_match_ids.is_empty());
        assert_eq!(h.newest_match_id(), None);
    }

    #[test]
    fn headshot_percent_matches_hand_computed_capture() {
        // Sanitized from the real match-details capture for the own player: across all
        // rounds head=9, body=26, leg=1 (total 36) -> 25%. Two rounds here reproduce the
        // same totals; the "other" subject's damage must be ignored.
        let payload = json!({ "roundResults": [
            { "playerStats": [
                { "subject": "me", "damage": [
                    { "headshots": 5, "bodyshots": 10, "legshots": 1 },
                    { "headshots": 0, "bodyshots": 6, "legshots": 0 }
                ]},
                { "subject": "other", "damage": [
                    { "headshots": 99, "bodyshots": 0, "legshots": 0 }
                ]}
            ]},
            { "playerStats": [
                { "subject": "me", "damage": [
                    { "headshots": 4, "bodyshots": 10, "legshots": 0 }
                ]}
            ]}
        ]});
        let mut acc = HitCounts::default();
        accumulate_match_hits(&mut acc, &payload, "me");
        assert_eq!(acc, HitCounts { head: 9, body: 26, leg: 1 });
        assert_eq!(acc.total(), 36);
        assert_eq!(acc.headshot_percent(), Some(25));
    }

    #[test]
    fn headshot_percent_accumulates_across_matches() {
        let m1 = json!({ "roundResults": [ { "playerStats": [
            { "subject": "me", "damage": [ { "headshots": 1, "bodyshots": 1, "legshots": 0 } ] }
        ]}]});
        let m2 = json!({ "roundResults": [ { "playerStats": [
            { "subject": "me", "damage": [ { "headshots": 1, "bodyshots": 0, "legshots": 0 } ] }
        ]}]});
        let mut acc = HitCounts::default();
        accumulate_match_hits(&mut acc, &m1, "me");
        accumulate_match_hits(&mut acc, &m2, "me");
        // head 2, body 1, leg 0 -> 2/3 = 66.67 -> 67
        assert_eq!(acc.headshot_percent(), Some(67));
    }

    #[test]
    fn headshot_percent_rounds_half_to_even() {
        // 1 head of 8 total = 12.5 (an exact tie) -> round-half-to-even -> 12, matching
        // Python round() (a round-half-up would wrongly give 13).
        let acc = HitCounts { head: 1, body: 7, leg: 0 };
        assert_eq!(acc.total(), 8);
        assert_eq!(acc.headshot_percent(), Some(12));
    }

    #[test]
    fn no_hits_is_none() {
        let acc = HitCounts::default();
        assert_eq!(acc.headshot_percent(), None);
        // A payload with no damage for the subject leaves the accumulator empty.
        let payload = json!({ "roundResults": [ { "playerStats": [
            { "subject": "someone-else", "damage": [ { "headshots": 3, "bodyshots": 0, "legshots": 0 } ] }
        ]}]});
        let mut acc2 = HitCounts::default();
        accumulate_match_hits(&mut acc2, &payload, "me");
        assert_eq!(acc2.headshot_percent(), None);
    }
}
