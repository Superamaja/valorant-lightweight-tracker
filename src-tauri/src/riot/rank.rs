//! MMR parsing + current/peak rank computation, including the pre-Ascendant tier shift.
//! Pure functions over the parsed `/mmr/v1/players/{puuid}` payload.

use crate::riot::constants::{BEFORE_ASCENDANT_SEASONS, NUMBER_TO_RANK};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

/// Tier number -> rank name. Saturates at 27 (Radiant) and clamps 0 for unknown.
pub fn tier_name(tier: u8) -> &'static str {
    let idx = (tier as usize).min(NUMBER_TO_RANK.len() - 1);
    NUMBER_TO_RANK[idx]
}

/// Parsed MMR payload (only the fields we use).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MmrResponse {
    #[serde(rename = "QueueSkills", default)]
    pub queue_skills: QueueSkills,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct QueueSkills {
    #[serde(default)]
    pub competitive: Competitive,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Competitive {
    #[serde(rename = "SeasonalInfoBySeasonID", default)]
    pub seasonal: HashMap<String, SeasonalInfo>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SeasonalInfo {
    #[serde(rename = "CompetitiveTier", default)]
    pub competitive_tier: u8,
    #[serde(rename = "RankedRating", default)]
    pub ranked_rating: i32,
    #[serde(rename = "LeaderboardRank", default)]
    pub leaderboard_rank: i32,
    #[serde(rename = "WinsByTier", default)]
    pub wins_by_tier: Option<HashMap<String, i64>>,
    #[serde(rename = "NumberOfWins", default)]
    pub number_of_wins: u32,
    #[serde(rename = "NumberOfGames", default)]
    pub number_of_games: u32,
}

/// Current-season rank result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurrentRank {
    pub tier: u8,
    pub rr: i32,
    pub leaderboard_rank: i32,
}

impl CurrentRank {
    pub fn unranked() -> Self {
        Self { tier: 0, rr: 0, leaderboard_rank: 0 }
    }
}

/// Peak-rank result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeakRank {
    pub tier: u8,
    /// Season UUID the peak was achieved in (None if peak == current with no wins data).
    pub season_id: Option<String>,
}

/// Parse a raw MMR JSON value into `MmrResponse`. Consumes the value (no deep clone).
/// Never fails — missing/odd fields degrade to defaults so a single bad row can't break
/// the table (pitfall §14).
pub fn parse_mmr(value: Value) -> MmrResponse {
    serde_json::from_value(value).unwrap_or_default()
}

/// Current rank / RR / leaderboard for `season_id`. Follows vRY `rank.get_rank`:
/// - tier >= 21 (Ascendant+): tier, RR, leaderboard
/// - tier 3..=20 (Iron..Diamond): tier, RR, leaderboard 0
/// - tier 0/1/2 or missing season: Unranked (0/0/0)
pub fn compute_current(mmr: &MmrResponse, season_id: &str) -> CurrentRank {
    let Some(info) = mmr.queue_skills.competitive.seasonal.get(season_id) else {
        return CurrentRank::unranked();
    };
    let tier = info.competitive_tier;
    if tier >= 21 {
        CurrentRank { tier, rr: info.ranked_rating, leaderboard_rank: info.leaderboard_rank }
    } else if !matches!(tier, 0..=2) {
        CurrentRank { tier, rr: info.ranked_rating, leaderboard_rank: 0 }
    } else {
        CurrentRank::unranked()
    }
}

/// Current-season win rate from the same MMR payload used for ranks (no extra request).
/// vRY shows `NumberOfWins / NumberOfGames` for the active act (probe: 8/14 -> "57 (14)").
/// Returns `None` when the season is absent or the player has 0 games (vRY shows "N/A").
pub fn compute_win_rate(
    mmr: &MmrResponse,
    season_id: &str,
) -> Option<crate::riot::types::WinRate> {
    let info = mmr.queue_skills.competitive.seasonal.get(season_id)?;
    if info.number_of_games == 0 {
        return None;
    }
    // Round half-to-even (banker's rounding) to match Python's `round()` exactly — that is
    // what vRY uses, so 12.5 -> 12 (not 13). `round()` would round half away from zero and
    // disagree on tie values.
    let percent = ((info.number_of_wins as f64 / info.number_of_games as f64) * 100.0)
        .round_ties_even() as u32;
    Some(crate::riot::types::WinRate { percent, games: info.number_of_games })
}

/// Peak rank across every recorded season. Starts from `current_tier` and scans each
/// season's `WinsByTier` for the highest tier with > 0 wins, applying the +3 shift for
/// pre-Ascendant seasons so old Immortal/Radiant wins don't collide with Ascendant.
///
/// Seasons are scanned in content-service (chronological) order so a tie on the peak tier
/// attributes deterministically to the earliest season, matching vRY's first-wins walk over
/// Riot's insertion-ordered payload. Ids missing from `seasons` sort last, by id.
pub fn compute_peak(
    mmr: &MmrResponse,
    current_tier: u8,
    seasons: &[crate::riot::content::Season],
) -> PeakRank {
    let mut max_tier = current_tier;
    let mut max_season: Option<String> = None;

    let position = |id: &str| {
        seasons
            .iter()
            .position(|s| s.id.eq_ignore_ascii_case(id))
            .unwrap_or(usize::MAX)
    };
    let mut entries: Vec<_> = mmr.queue_skills.competitive.seasonal.iter().collect();
    entries.sort_by(|(a, _), (b, _)| position(a).cmp(&position(b)).then_with(|| a.cmp(b)));

    for (season_id, info) in entries {
        let Some(wins) = &info.wins_by_tier else { continue };
        let pre_ascendant = BEFORE_ASCENDANT_SEASONS.contains(&season_id.as_str());
        for (tier_key, win_count) in wins {
            if *win_count <= 0 {
                continue;
            }
            let Ok(mut tier_num) = tier_key.parse::<i32>() else { continue };
            if pre_ascendant && tier_num > 20 {
                tier_num += 3;
            }
            let tier_num = tier_num.clamp(0, 27) as u8;
            if tier_num > max_tier {
                max_tier = tier_num;
                max_season = Some(season_id.clone());
            }
        }
    }

    PeakRank { tier: max_tier, season_id: max_season }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tier_names_map_correctly() {
        assert_eq!(tier_name(24), "Immortal 1");
        assert_eq!(tier_name(200), "Radiant"); // saturates
    }

    fn mmr_with(season: &str, tier: u8, rr: i32, lb: i32, wins: Value) -> MmrResponse {
        parse_mmr(json!({
            "QueueSkills": { "competitive": { "SeasonalInfoBySeasonID": {
                season: { "CompetitiveTier": tier, "RankedRating": rr,
                          "LeaderboardRank": lb, "WinsByTier": wins }
            }}}
        }))
    }

    fn mmr_with_wr(season: &str, wins: u32, games: u32) -> MmrResponse {
        parse_mmr(json!({
            "QueueSkills": { "competitive": { "SeasonalInfoBySeasonID": {
                season: { "CompetitiveTier": 21, "NumberOfWins": wins, "NumberOfGames": games }
            }}}
        }))
    }

    #[test]
    fn win_rate_from_wins_and_games() {
        for (wins, games, season, expected) in [
            // Sanitized from the live capture: 8 wins / 14 games -> vRY "57 (14)".
            (8, 14, "s", Some((57, 14))),
            (0, 0, "s", None),
            (5, 10, "other", None),
            // 1/3 = 33.33 -> 33
            (1, 3, "s", Some((33, 3))),
            // 2/3 = 66.67 -> 67
            (2, 3, "s", Some((67, 3))),
            // 1/8 = 12.5 (an exact tie) -> round-half-to-even -> 12, matching Python round()
            // (a round-half-up would wrongly give 13).
            (1, 8, "s", Some((12, 8))),
        ] {
            let wr = compute_win_rate(&mmr_with_wr("s", wins, games), season);
            assert_eq!(wr.map(|wr| (wr.percent, wr.games)), expected, "{wins}/{games} in {season}");
        }
    }

    #[test]
    fn current_rank_per_tier_and_season() {
        for (tier, rr, lb, season, expected) in [
            // Immortal shows RR and leaderboard.
            (25, 340, 128, "s1", CurrentRank { tier: 25, rr: 340, leaderboard_rank: 128 }),
            // Gold hides the leaderboard.
            (13, 55, 999, "s1", CurrentRank { tier: 13, rr: 55, leaderboard_rank: 0 }),
            // An unranked tier is zeroed.
            (2, 40, 0, "s1", CurrentRank::unranked()),
            // A missing season is unranked.
            (20, 40, 0, "other", CurrentRank::unranked()),
        ] {
            let mmr = mmr_with("s1", tier, rr, lb, json!(null));
            assert_eq!(compute_current(&mmr, season), expected, "tier {tier} in {season}");
        }
    }

    #[test]
    fn peak_from_a_single_season() {
        let old_season = BEFORE_ASCENDANT_SEASONS[0];
        for (season, current, wins, expected_tier, expected_season) in [
            // current Diamond(18); a season with Immortal-1 (24) wins -> peak 24.
            ("modern-season", 18, json!({ "24": 3, "20": 10 }), 24, Some("modern-season")),
            // Zero-win tiers are ignored.
            ("modern-season", 18, json!({ "27": 0, "22": 5 }), 22, Some("modern-season")),
            // Pre-Ascendant season: Immortal was tier 21 back then; +3 -> 24 (modern Immortal 1).
            (old_season, 15, json!({ "21": 4 }), 24, Some(old_season)),
            // Modern tier 21 (Ascendant 1) is unchanged.
            ("modern-season", 15, json!({ "21": 4 }), 21, Some("modern-season")),
            // No wins: the peak defaults to the current tier.
            ("s1", 19, json!(null), 19, None),
        ] {
            let peak = compute_peak(&mmr_with(season, current, 0, 0, wins), current, &[]);
            assert_eq!((peak.tier, peak.season_id.as_deref()), (expected_tier, expected_season), "{season}");
        }
    }

    #[test]
    fn peak_tie_attributes_to_earliest_content_season() {
        // Same peak tier (22) reached in two acts: the tie must go to whichever season
        // comes first in the content-service list, regardless of HashMap iteration order.
        let mmr = parse_mmr(json!({
            "QueueSkills": { "competitive": { "SeasonalInfoBySeasonID": {
                "act-late":  { "CompetitiveTier": 20, "WinsByTier": { "22": 2 } },
                "act-early": { "CompetitiveTier": 20, "WinsByTier": { "22": 5 } }
            }}}
        }));
        let season = |id: &str| crate::riot::content::Season {
            id: id.into(),
            name: String::new(),
            season_type: "act".into(),
            start_time: String::new(),
            end_time: String::new(),
            is_active: false,
        };
        let seasons = [season("act-early"), season("act-late")];
        for _ in 0..16 {
            let peak = compute_peak(&mmr, 18, &seasons);
            assert_eq!(peak.tier, 22);
            assert_eq!(peak.season_id.as_deref(), Some("act-early"));
        }
    }
}
