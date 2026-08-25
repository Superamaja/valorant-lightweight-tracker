//! Phase 2 per-player stats parsing: competitiveupdates (ΔRR + last-N W/L pips + the
//! recent match ids HS%/KD reuse) and match-details head/body/leg + kill/death accounting.
//! All pure and fixture-testable; the network fetching lives in `remote_api`, the
//! orchestration/caching in `app_state`.

use crate::riot::constants::{RECENT_MATCHES_FOR_HS, RECENT_RESULTS_COUNT};
use crate::riot::types::MatchResult;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

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

// --- match-details accounting (HS% + KD) ------------------------------------

/// The per-player figures derived from the recent match-details payloads. Both come from
/// the same downloads, so they travel together through the cache and into the row.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RecentStats {
    /// Headshot percent 0-100, or None ("N/a").
    pub headshot_percent: Option<u32>,
    /// Kills/deaths over the same window, 2 decimals, or None.
    pub kd: Option<f64>,
}

/// Running totals for one player across one or more match-details payloads: head/body/leg
/// hits (HS%) plus kills/deaths (KD). One accumulator because one pass over the same
/// payloads fills both — KD costs no extra request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MatchTotals {
    pub head: u64,
    pub body: u64,
    pub leg: u64,
    pub kills: u64,
    pub deaths: u64,
    /// Matches whose `players[]` carried a `stats` entry for the subject — 0 means "no KD".
    pub kd_matches: u32,
}

/// What one match-details payload contributes to each player who appears in it, keyed by
/// puuid. This is all the caller keeps of a ~500 KB response.
pub type MatchContribution = HashMap<String, MatchTotals>;

impl MatchTotals {
    /// Fold another match's totals into this window.
    pub fn add(&mut self, other: MatchTotals) {
        self.head += other.head;
        self.body += other.body;
        self.leg += other.leg;
        self.kills += other.kills;
        self.deaths += other.deaths;
        self.kd_matches += other.kd_matches;
    }

    pub fn total_hits(&self) -> u64 {
        self.head + self.body + self.leg
    }

    /// Headshot percent = round(head / (head+body+leg) * 100). `None` when no hits recorded
    /// (vRY shows "N/a") — mirrors vRY `player_stats._process_match_data`. Uses round
    /// half-to-even to match Python's `round()` exactly (e.g. 12.5 -> 12, not 13).
    pub fn headshot_percent(&self) -> Option<u32> {
        let total = self.total_hits();
        if total == 0 {
            return None;
        }
        Some(((self.head as f64 / total as f64) * 100.0).round_ties_even() as u32)
    }

    /// Kills/deaths across the window, rounded to 2 decimals. Zero deaths yields the kill
    /// count itself (7 kills, 0 deaths -> 7.0). `None` when no match carried stats for the
    /// player — the same "no data" case HS% reports as `None`.
    pub fn kd(&self) -> Option<f64> {
        if self.kd_matches == 0 {
            return None;
        }
        let ratio = if self.deaths == 0 {
            self.kills as f64
        } else {
            self.kills as f64 / self.deaths as f64
        };
        Some((ratio * 100.0).round() / 100.0)
    }

    /// The display-ready pair this player's recent matches yield.
    pub fn recent_stats(&self) -> RecentStats {
        RecentStats { headshot_percent: self.headshot_percent(), kd: self.kd() }
    }
}

/// Reduce one match-details payload to every player's totals: head/body/leg hits summed over
/// every damage entry of every round's `playerStats` (replicating vRY `player_stats.py`), plus
/// the kills/deaths the top-level `players[]` entry already carries. Every player is read in
/// the one pass, so a match several lobby members played is parsed — and downloaded — once.
/// Borrows `value` (no deep clone). Missing fields default to 0.
pub fn match_contribution(value: &Value) -> MatchContribution {
    let mut out = MatchContribution::new();
    collect_hits(&mut out, value);
    collect_kills_deaths(&mut out, value);
    out
}

fn collect_hits(out: &mut MatchContribution, value: &Value) {
    let Some(rounds) = value.get("roundResults").and_then(|r| r.as_array()) else {
        return;
    };
    for round in rounds {
        let Some(player_stats) = round.get("playerStats").and_then(|p| p.as_array()) else {
            continue;
        };
        for ps in player_stats {
            let Some(subject) = ps.get("subject").and_then(|s| s.as_str()) else {
                continue;
            };
            let Some(damage) = ps.get("damage").and_then(|d| d.as_array()) else {
                continue;
            };
            let acc = out.entry(subject.to_string()).or_default();
            for d in damage {
                acc.head += d.get("headshots").and_then(|v| v.as_u64()).unwrap_or(0);
                acc.body += d.get("bodyshots").and_then(|v| v.as_u64()).unwrap_or(0);
                acc.leg += d.get("legshots").and_then(|v| v.as_u64()).unwrap_or(0);
            }
        }
    }
}

/// The match-wide totals live once per player in the payload's top-level `players[]`, each
/// entry a `subject` puuid plus a `stats` object with `kills`/`deaths`.
fn collect_kills_deaths(out: &mut MatchContribution, value: &Value) {
    let Some(players) = value.get("players").and_then(|p| p.as_array()) else {
        return;
    };
    for player in players {
        let Some(subject) = player.get("subject").and_then(|s| s.as_str()) else {
            continue;
        };
        let Some(player_stats) = player.get("stats") else {
            continue;
        };
        let acc = out.entry(subject.to_string()).or_default();
        acc.kills += player_stats.get("kills").and_then(|v| v.as_u64()).unwrap_or(0);
        acc.deaths += player_stats.get("deaths").and_then(|v| v.as_u64()).unwrap_or(0);
        acc.kd_matches += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// What one payload contributes to `subject`'s window, the way a caller folds it in.
    fn totals_for(value: &Value, subject: &str) -> MatchTotals {
        match_contribution(value).get(subject).copied().unwrap_or_default()
    }

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
        let mut acc = MatchTotals::default();
        acc.add(totals_for(&payload, "me"));
        assert_eq!(acc.head, 9);
        assert_eq!(acc.body, 26);
        assert_eq!(acc.leg, 1);
        assert_eq!(acc.total_hits(), 36);
        assert_eq!(acc.headshot_percent(), Some(25));
        // No top-level players[] in this fixture -> no KD.
        assert_eq!(acc.kd(), None);
    }

    #[test]
    fn headshot_percent_accumulates_across_matches() {
        let m1 = json!({ "roundResults": [ { "playerStats": [
            { "subject": "me", "damage": [ { "headshots": 1, "bodyshots": 1, "legshots": 0 } ] }
        ]}]});
        let m2 = json!({ "roundResults": [ { "playerStats": [
            { "subject": "me", "damage": [ { "headshots": 1, "bodyshots": 0, "legshots": 0 } ] }
        ]}]});
        let mut acc = MatchTotals::default();
        acc.add(totals_for(&m1, "me"));
        acc.add(totals_for(&m2, "me"));
        // head 2, body 1, leg 0 -> 2/3 = 66.67 -> 67
        assert_eq!(acc.headshot_percent(), Some(67));
    }

    #[test]
    fn headshot_percent_rounds_half_to_even() {
        // 1 head of 8 total = 12.5 (an exact tie) -> round-half-to-even -> 12, matching
        // Python round() (a round-half-up would wrongly give 13).
        let acc = MatchTotals { head: 1, body: 7, leg: 0, ..Default::default() };
        assert_eq!(acc.total_hits(), 8);
        assert_eq!(acc.headshot_percent(), Some(12));
    }

    #[test]
    fn no_hits_is_none() {
        let acc = MatchTotals::default();
        assert_eq!(acc.headshot_percent(), None);
        // A payload with no damage for the subject leaves the accumulator empty.
        let payload = json!({ "roundResults": [ { "playerStats": [
            { "subject": "someone-else", "damage": [ { "headshots": 3, "bodyshots": 0, "legshots": 0 } ] }
        ]}]});
        let mut acc2 = MatchTotals::default();
        acc2.add(totals_for(&payload, "me"));
        assert_eq!(acc2.headshot_percent(), None);
    }

    /// A match-details payload carrying just the top-level per-player kill/death stats.
    fn kd_payload(entries: &[(&str, u64, u64)]) -> Value {
        let players: Vec<Value> = entries
            .iter()
            .map(|(subject, kills, deaths)| {
                json!({ "subject": subject, "stats": { "kills": kills, "deaths": deaths } })
            })
            .collect();
        json!({ "players": players })
    }

    #[test]
    fn kd_accumulates_across_matches_and_ignores_other_players() {
        // 20/15 then 17/14 for "me" -> 37 kills / 29 deaths = 1.2758... -> 1.28.
        let m1 = kd_payload(&[("me", 20, 15), ("other", 99, 1)]);
        let m2 = kd_payload(&[("me", 17, 14)]);
        let mut acc = MatchTotals::default();
        acc.add(totals_for(&m1, "me"));
        acc.add(totals_for(&m2, "me"));
        assert_eq!(acc.kills, 37);
        assert_eq!(acc.deaths, 29);
        assert_eq!(acc.kd_matches, 2);
        assert_eq!(acc.kd(), Some(1.28));
    }

    #[test]
    fn one_parse_yields_every_players_totals() {
        // The whole payload is read in one pass, so two lobby members who played the same
        // match both come out of a single download.
        let payload = json!({
            "players": [
                { "subject": "a", "stats": { "kills": 20, "deaths": 15 } },
                { "subject": "b", "stats": { "kills": 9, "deaths": 18 } }
            ],
            "roundResults": [ { "playerStats": [
                { "subject": "a", "damage": [ { "headshots": 2, "bodyshots": 2, "legshots": 0 } ] },
                { "subject": "b", "damage": [ { "headshots": 0, "bodyshots": 4, "legshots": 0 } ] }
            ]}]
        });
        let contribution = match_contribution(&payload);
        assert_eq!(contribution.len(), 2);
        assert_eq!(contribution["a"].headshot_percent(), Some(50));
        assert_eq!(contribution["a"].kd(), Some(1.33));
        assert_eq!(contribution["b"].headshot_percent(), Some(0));
        assert_eq!(contribution["b"].kd(), Some(0.5));
        // A player who never appeared is simply absent.
        assert!(!contribution.contains_key("c"));
    }

    #[test]
    fn kd_rounds_to_two_decimals() {
        let mut acc = MatchTotals::default();
        acc.add(totals_for(&kd_payload(&[("me", 2, 3)]), "me"));
        // 0.6666... -> 0.67
        assert_eq!(acc.kd(), Some(0.67));
    }

    #[test]
    fn kd_with_zero_deaths_is_the_kill_count() {
        let mut acc = MatchTotals::default();
        acc.add(totals_for(&kd_payload(&[("me", 7, 0)]), "me"));
        assert_eq!(acc.kd(), Some(7.0));
    }

    #[test]
    fn kd_without_stats_for_the_player_is_none() {
        // No matches at all.
        assert_eq!(MatchTotals::default().kd(), None);
        // Matches processed, but none carried a stats entry for this player.
        let mut acc = MatchTotals::default();
        acc.add(totals_for(&kd_payload(&[("someone-else", 30, 2)]), "me"));
        assert_eq!(acc.kd_matches, 0);
        assert_eq!(acc.kd(), None);
        // A players[] entry without a stats object is not counted either.
        let mut acc2 = MatchTotals::default();
        acc2.add(totals_for(&json!({ "players": [ { "subject": "me" } ] }), "me"));
        assert_eq!(acc2.kd(), None);
    }

    #[test]
    fn recent_stats_carries_both_figures_from_one_pass() {
        // One payload with both the round damage (HS%) and the top-level kills/deaths (KD).
        let payload = json!({
            "players": [ { "subject": "me", "stats": { "kills": 3, "deaths": 2 } } ],
            "roundResults": [ { "playerStats": [
                { "subject": "me", "damage": [ { "headshots": 1, "bodyshots": 1, "legshots": 0 } ] }
            ]}]
        });
        let mut acc = MatchTotals::default();
        acc.add(totals_for(&payload, "me"));
        let stats = acc.recent_stats();
        assert_eq!(stats.headshot_percent, Some(50));
        assert_eq!(stats.kd, Some(1.5));
    }
}
