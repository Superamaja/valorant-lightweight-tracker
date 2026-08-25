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
        assert_eq!(tier_name(0), "Unranked");
        assert_eq!(tier_name(3), "Iron 1");
        assert_eq!(tier_name(21), "Ascendant 1");
        assert_eq!(tier_name(24), "Immortal 1");
        assert_eq!(tier_name(27), "Radiant");
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
    fn win_rate_matches_vry_probe() {
        // Sanitized from the live capture: 8 wins / 14 games -> vRY "57 (14)".
        let mmr = mmr_with_wr("s1", 8, 14);
        let wr = compute_win_rate(&mmr, "s1").unwrap();
        assert_eq!(wr.percent, 57);
        assert_eq!(wr.games, 14);
    }

    #[test]
    fn win_rate_is_none_with_zero_games() {
        let mmr = mmr_with_wr("s1", 0, 0);
        assert!(compute_win_rate(&mmr, "s1").is_none());
    }

    #[test]
    fn win_rate_is_none_when_season_missing() {
        let mmr = mmr_with_wr("s1", 5, 10);
        assert!(compute_win_rate(&mmr, "other").is_none());
    }

    #[test]
    fn win_rate_rounds_to_nearest_percent() {
        // 1/3 = 33.33 -> 33
        assert_eq!(compute_win_rate(&mmr_with_wr("s", 1, 3), "s").unwrap().percent, 33);
        // 2/3 = 66.67 -> 67
        assert_eq!(compute_win_rate(&mmr_with_wr("s", 2, 3), "s").unwrap().percent, 67);
    }

    #[test]
    fn win_rate_rounds_half_to_even() {
        // 1/8 = 12.5 (an exact tie) -> round-half-to-even -> 12, matching Python round()
        // (a round-half-up would wrongly give 13).
        assert_eq!(compute_win_rate(&mmr_with_wr("s", 1, 8), "s").unwrap().percent, 12);
    }

    #[test]
    fn current_immortal_shows_rr_and_leaderboard() {
        let mmr = mmr_with("s1", 25, 340, 128, json!(null));
        let cur = compute_current(&mmr, "s1");
        assert_eq!(cur, CurrentRank { tier: 25, rr: 340, leaderboard_rank: 128 });
    }

    #[test]
    fn current_gold_hides_leaderboard() {
        let mmr = mmr_with("s1", 13, 55, 999, json!(null));
        let cur = compute_current(&mmr, "s1");
        assert_eq!(cur, CurrentRank { tier: 13, rr: 55, leaderboard_rank: 0 });
    }

    #[test]
    fn current_unranked_tier_is_zeroed() {
        let mmr = mmr_with("s1", 2, 40, 0, json!(null));
        assert_eq!(compute_current(&mmr, "s1"), CurrentRank::unranked());
    }

    #[test]
    fn current_missing_season_is_unranked() {
        let mmr = mmr_with("s1", 20, 40, 0, json!(null));
        assert_eq!(compute_current(&mmr, "other"), CurrentRank::unranked());
    }

    #[test]
    fn peak_takes_highest_wins_tier() {
        // current Diamond(18); a season with Immortal-1 (24) wins -> peak 24.
        let mmr = mmr_with("modern-season", 18, 20, 0, json!({ "24": 3, "20": 10 }));
        let peak = compute_peak(&mmr, 18, &[]);
        assert_eq!(peak.tier, 24);
        assert_eq!(peak.season_id.as_deref(), Some("modern-season"));
    }

    #[test]
    fn peak_ignores_zero_win_tiers() {
        let mmr = mmr_with("modern-season", 18, 20, 0, json!({ "27": 0, "22": 5 }));
        let peak = compute_peak(&mmr, 18, &[]);
        assert_eq!(peak.tier, 22);
    }

    #[test]
    fn peak_applies_before_ascendant_shift() {
        // Pre-Ascendant season: Immortal was tier 21 back then; +3 -> 24 (modern Immortal 1).
        let old_season = BEFORE_ASCENDANT_SEASONS[0];
        let mmr = mmr_with(old_season, 15, 0, 0, json!({ "21": 4 }));
        let peak = compute_peak(&mmr, 15, &[]);
        assert_eq!(peak.tier, 24, "old tier 21 (Immortal) must shift to 24");
    }

    #[test]
    fn peak_no_shift_for_modern_season() {
        let mmr = mmr_with("modern-season", 15, 0, 0, json!({ "21": 4 }));
        let peak = compute_peak(&mmr, 15, &[]);
        assert_eq!(peak.tier, 21, "modern tier 21 (Ascendant 1) unchanged");
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

    #[test]
    fn peak_defaults_to_current_when_no_wins() {
        let mmr = mmr_with("s1", 19, 30, 0, json!(null));
        let peak = compute_peak(&mmr, 19, &[]);
        assert_eq!(peak.tier, 19);
        assert!(peak.season_id.is_none());
    }
}
