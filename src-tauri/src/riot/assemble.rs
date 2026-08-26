//! Assemble display-ready `PlayerRow`s from the fetched match/name/rank data.
//! Pure and fixture-testable — this is where privacy rules (incognito, HideAccountLevel)
//! and rank resolution are applied. See spec §5, §7, §8.

use crate::riot::content::{self, Season};
use crate::riot::loadout::PlayerSkinIds;
use crate::riot::match_state::MatchPlayer;
use crate::riot::rank::{self, MmrResponse};
use crate::riot::static_data::StaticData;
use crate::riot::stats::{RecentStats, RrHistory};
use crate::riot::types::{PendingStats, PlayerRow};
use std::collections::HashMap;

/// All the resolved inputs needed to build the table.
pub struct AssembleInput<'a> {
    pub players: &'a [MatchPlayer],
    /// puuid -> "GameName#TagLine".
    pub names: &'a HashMap<String, String>,
    /// puuid -> parsed MMR (missing entries -> Unranked). Also the WR source (phase 2).
    pub mmr: &'a HashMap<String, MmrResponse>,
    /// puuid -> partyId (only real parties, size > 1).
    pub parties: &'a HashMap<String, String>,
    /// puuid -> ΔRR + last-5 pips (phase 2; missing -> no history).
    pub updates: &'a HashMap<String, RrHistory>,
    /// puuid -> HS% + KD over recent matches (phase 2; missing / inner None -> null).
    pub recent_stats: &'a HashMap<String, RecentStats>,
    /// puuid -> equipped Vandal/Phantom skin uuids (phase 2, INGAME only; empty in pregame).
    pub skins: &'a HashMap<String, PlayerSkinIds>,
    pub static_data: &'a StaticData,
    pub own_puuid: &'a str,
    pub own_team: Option<&'a str>,
    pub current_season_id: &'a str,
    /// Content-service season list, used to label the act a peak rank was achieved in.
    pub seasons: &'a [Season],
    /// Whether this build is the settled one for the match. A settled build carries no
    /// pending flags: an absent value is then genuinely absent, not still in flight.
    pub finality: bool,
    /// Whether the match's loadouts have settled. Loadouts arrive for the whole roster at
    /// once, so this is per-match, not per-row.
    pub loadouts_fetched: bool,
    /// Whether the match is INGAME — the only state where loadouts exist at all.
    pub ingame: bool,
}

/// Build the player rows. Applies incognito/level privacy rules, rank + phase-2 stat
/// resolution, and the guaranteed row ordering (see `order_rows`).
pub fn assemble_players(input: &AssembleInput) -> Vec<PlayerRow> {
    let own_party = input.parties.get(input.own_puuid).cloned();
    let default_mmr = MmrResponse::default();

    let mut rows: Vec<PlayerRow> = input
        .players
        .iter()
        .map(|p| {
            let is_self = p.puuid == input.own_puuid;
            let is_ally = input.own_team.map(|t| t == p.team).unwrap_or(false);
            let player_party = input.parties.get(&p.puuid).cloned();
            let shares_party = matches!((&own_party, &player_party), (Some(a), Some(b)) if a == b);
            let is_party_of_self = shares_party && !is_self;

            // Name: hidden for incognito players (except yourself).
            let name = if p.incognito && !is_self {
                None
            } else {
                input.names.get(&p.puuid).cloned()
            };

            // Account level: shown to self + own party regardless; otherwise withheld when
            // the player is incognito OR set "hide my level" (contract: level is withheld for
            // incognito players too, not just the hide-level flag). A wire value of 0 is
            // Riot's own zeroing of a hidden level (it does this even on your own row), never
            // a real level, so it is withheld too.
            let level_visible =
                (!p.incognito && !p.hide_account_level) || is_self || is_party_of_self;
            let account_level =
                (level_visible && p.account_level > 0).then_some(p.account_level);

            // Ranks + WR (all from the same MMR payload — no extra request for WR).
            let mmr = input.mmr.get(&p.puuid).unwrap_or(&default_mmr);
            let current = rank::compute_current(mmr, input.current_season_id);
            let peak = rank::compute_peak(mmr, current.tier, input.seasons);
            // A peak that fell back to the current tier carries no season of its own; it was
            // reached in the act being played, so label it with that one.
            let peak_season = peak.season_id.as_deref().unwrap_or(input.current_season_id);
            let peak_rank_act = content::act_label(input.seasons, peak_season);
            let win_rate = rank::compute_win_rate(mmr, input.current_season_id);

            // ΔRR + last-5 pips.
            let history = input.updates.get(&p.puuid);
            let rr_change = history.and_then(|h| h.rr_change);
            let recent_results = history.map(|h| h.results.clone()).unwrap_or_default();

            // HS% + KD, both from the same recent match-details (missing entry or inner
            // None -> null).
            let recent = input.recent_stats.get(&p.puuid).copied().unwrap_or_default();

            // Skins (INGAME only; resolved uuid -> name/icon via static data).
            let player_skins = input.skins.get(&p.puuid);
            let vandal_skin = player_skins.and_then(|s| {
                input
                    .static_data
                    .skin(s.vandal.skin.as_deref(), s.vandal.chroma.as_deref())
            });
            let phantom_skin = player_skins.and_then(|s| {
                input
                    .static_data
                    .skin(s.phantom.skin.as_deref(), s.phantom.chroma.as_deref())
            });

            // A group is in flight while the data it is built from has not reached the cache,
            // up until the build that settles the match.
            let pending = PendingStats {
                name: !input.finality && !input.names.contains_key(&p.puuid),
                rank: !input.finality && !input.mmr.contains_key(&p.puuid),
                history: !input.finality && !input.updates.contains_key(&p.puuid),
                recent_stats: !input.finality && !input.recent_stats.contains_key(&p.puuid),
                skins: !input.finality && input.ingame && !input.loadouts_fetched,
            };

            PlayerRow {
                id: p.puuid.clone(),
                name,
                incognito: p.incognito,
                team: p.team.clone(),
                is_ally,
                is_self,
                agent: input.static_data.agent(p.character_id.as_deref()),
                agent_selection_state: p.selection_state.clone(),
                current_rank: input.static_data.rank(current.tier),
                rr: current.rr,
                leaderboard_rank: current.leaderboard_rank,
                peak_rank: input.static_data.rank(peak.tier),
                peak_rank_act,
                account_level,
                party_id: player_party,
                win_rate,
                rr_change,
                recent_results,
                headshot_percent: recent.headshot_percent,
                kd: recent.kd,
                vandal_skin,
                phantom_skin,
                pending,
            }
        })
        .collect();

    order_rows(&mut rows);
    rows
}

/// Impose the guaranteed row order the UI relies on (backend owns ordering, not the UI):
/// 1. the local player's team (`is_ally`) first, then enemies;
/// 2. within the ally block, the local player (`is_self`) first;
/// 3. deterministic tiebreak within each block: by display name (case-insensitive), with
///    hidden/unresolved names (`None`, e.g. incognito) sorted last, then by puuid.
///
/// The UI colours purely by `is_ally` (ally block = blue) — it never keys off the raw
/// Red/Blue `team` id, so this ordering plus `is_ally` is the whole contract.
///
/// Uses `sort_by_cached_key` so each row's key (notably the lowercased name) is computed
/// once, not on every pairwise comparison as the old `sort_by` did.
fn order_rows(rows: &mut [PlayerRow]) {
    rows.sort_by_cached_key(|r| {
        // Named rows before hidden/unresolved (None sorts last), then case-insensitive name.
        let name = match &r.name {
            Some(n) => (false, n.to_lowercase()),
            None => (true, String::new()),
        };
        (
            !r.is_ally,  // ally block before enemy block
            !r.is_self,  // self first (only ever within the ally block)
            name,        // then by display name
            r.id.clone(), // stable final tiebreak
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::riot::loadout::{EquippedSkin, PlayerSkinIds};
    use crate::riot::rank::parse_mmr;
    use serde_json::json;

    fn mmr_map(entries: &[(&str, serde_json::Value)]) -> HashMap<String, MmrResponse> {
        entries.iter().map(|(k, v)| (k.to_string(), parse_mmr(v.clone()))).collect()
    }

    fn base_player(puuid: &str, team: &str) -> MatchPlayer {
        MatchPlayer {
            puuid: puuid.into(),
            team: team.into(),
            character_id: None,
            selection_state: None,
            account_level: 100,
            incognito: false,
            hide_account_level: false,
        }
    }

    /// Test builder that fills the phase-2 inputs with empty maps by default so each test
    /// only supplies what it exercises.
    #[derive(Default)]
    struct Case {
        names: HashMap<String, String>,
        mmr: HashMap<String, MmrResponse>,
        parties: HashMap<String, String>,
        updates: HashMap<String, RrHistory>,
        recent_stats: HashMap<String, RecentStats>,
        skins: HashMap<String, PlayerSkinIds>,
        static_data: StaticData,
        own_team: Option<String>,
        seasons: Vec<Season>,
        finality: bool,
        loadouts_fetched: bool,
        ingame: bool,
    }

    impl Case {
        fn run(&self, players: &[MatchPlayer], own_puuid: &str) -> Vec<PlayerRow> {
            assemble_players(&AssembleInput {
                players,
                names: &self.names,
                mmr: &self.mmr,
                parties: &self.parties,
                updates: &self.updates,
                recent_stats: &self.recent_stats,
                skins: &self.skins,
                static_data: &self.static_data,
                own_puuid,
                own_team: self.own_team.as_deref(),
                current_season_id: "s1",
                seasons: &self.seasons,
                finality: self.finality,
                loadouts_fetched: self.loadouts_fetched,
                ingame: self.ingame,
            })
        }
    }

    fn row<'a>(rows: &'a [PlayerRow], id: &str) -> &'a PlayerRow {
        rows.iter().find(|r| r.id == id).expect("row present")
    }

    #[test]
    fn marks_self_and_ally() {
        let players = vec![base_player("me", "Blue"), base_player("foe", "Red")];
        let case = Case { own_team: Some("Blue".into()), ..Default::default() };
        let rows = case.run(&players, "me");
        let me = row(&rows, "me");
        let foe = row(&rows, "foe");
        assert!(me.is_self && me.is_ally);
        assert!(!foe.is_self && !foe.is_ally);
    }

    #[test]
    fn incognito_hides_name_but_not_for_self() {
        let mut me = base_player("me", "Blue");
        me.incognito = true;
        let mut foe = base_player("foe", "Red");
        foe.incognito = true;
        let players = vec![me, foe];
        let mut names = HashMap::new();
        names.insert("me".to_string(), "Me#1".to_string());
        names.insert("foe".to_string(), "Foe#2".to_string());
        let case = Case { names, own_team: Some("Blue".into()), ..Default::default() };
        let rows = case.run(&players, "me");
        assert_eq!(row(&rows, "me").name.as_deref(), Some("Me#1")); // self visible
        assert_eq!(row(&rows, "foe").name, None); // incognito enemy hidden
        assert!(row(&rows, "foe").incognito);
    }

    #[test]
    fn incognito_hides_account_level_except_for_self() {
        // Incognito withholds accountLevel too (contract), independent of hide_account_level.
        let mut me = base_player("me", "Blue");
        me.incognito = true;
        let mut foe = base_player("foe", "Red"); // incognito, hide_account_level == false
        foe.incognito = true;
        let players = vec![me, foe];
        let case = Case { own_team: Some("Blue".into()), ..Default::default() };
        let rows = case.run(&players, "me");
        assert_eq!(row(&rows, "me").account_level, Some(100)); // self always sees own level
        assert_eq!(row(&rows, "foe").account_level, None); // incognito enemy level withheld
    }

    #[test]
    fn hide_account_level_gated_by_party_and_self() {
        let mut foe = base_player("foe", "Red");
        foe.hide_account_level = true;
        let mut mate = base_player("mate", "Blue");
        mate.hide_account_level = true;
        let players = vec![foe, mate];
        let mut parties = HashMap::new();
        parties.insert("me".to_string(), "party".to_string());
        parties.insert("mate".to_string(), "party".to_string());
        let case = Case { parties, own_team: Some("Blue".into()), ..Default::default() };
        let rows = case.run(&players, "me");
        assert_eq!(row(&rows, "foe").account_level, None); // enemy hiding level -> blanked
        assert_eq!(row(&rows, "mate").account_level, Some(100)); // party member -> shown
    }

    #[test]
    fn resolves_current_and_peak_rank_and_win_rate() {
        let players = vec![base_player("me", "Blue")];
        let mmr = mmr_map(&[(
            "me",
            json!({ "QueueSkills": { "competitive": { "SeasonalInfoBySeasonID": {
                "s1": { "CompetitiveTier": 13, "RankedRating": 40, "LeaderboardRank": 0,
                        "WinsByTier": { "24": 2 }, "NumberOfWins": 8, "NumberOfGames": 14 }
            }}}}),
        )]);
        let case = Case { mmr, own_team: Some("Blue".into()), ..Default::default() };
        let rows = case.run(&players, "me");
        let me = row(&rows, "me");
        assert_eq!(me.current_rank.tier, 13);
        assert_eq!(me.current_rank.name, "Gold 2");
        assert_eq!(me.rr, 40);
        assert_eq!(me.peak_rank.tier, 24); // Immortal 1 from WinsByTier
        assert_eq!(me.peak_rank.name, "Immortal 1");
        // WR derived from the SAME mmr payload (no extra request).
        let wr = me.win_rate.unwrap();
        assert_eq!(wr.percent, 57);
        assert_eq!(wr.games, 14);
    }

    #[test]
    fn labels_the_act_the_peak_was_achieved_in() {
        let players = vec![base_player("me", "Blue")];
        let mmr = mmr_map(&[(
            "me",
            json!({ "QueueSkills": { "competitive": { "SeasonalInfoBySeasonID": {
                "e6a3": { "CompetitiveTier": 0, "WinsByTier": { "24": 2 } },
                "s1": { "CompetitiveTier": 13, "RankedRating": 40 }
            }}}}),
        )]);
        let seasons = crate::riot::content::parse_seasons(&json!({ "Seasons": [
            { "ID": "e6", "Name": "EPISODE 6", "Type": "episode" },
            { "ID": "e6a3", "Name": "ACT III", "Type": "act" },
            { "ID": "e7", "Name": "EPISODE 7", "Type": "episode" },
            { "ID": "s1", "Name": "ACT I", "Type": "act", "IsActive": true }
        ]}));
        let case = Case { mmr, seasons, own_team: Some("Blue".into()), ..Default::default() };
        let rows = case.run(&players, "me");
        let me = row(&rows, "me");
        assert_eq!(me.peak_rank.tier, 24);
        assert_eq!(me.peak_rank_act.as_deref(), Some("E6: A3"));
    }

    #[test]
    fn peak_act_falls_back_to_the_current_act() {
        // Peak == current tier (no wins data), so the peak belongs to the act being played.
        let players = vec![base_player("me", "Blue")];
        let seasons = crate::riot::content::parse_seasons(&json!({ "Seasons": [
            { "ID": "e7", "Name": "EPISODE 7", "Type": "episode" },
            { "ID": "s1", "Name": "ACT I", "Type": "act", "IsActive": true }
        ]}));
        let case = Case { seasons, own_team: Some("Blue".into()), ..Default::default() };
        let rows = case.run(&players, "me");
        assert_eq!(row(&rows, "me").peak_rank_act.as_deref(), Some("E7: A1"));
    }

    #[test]
    fn peak_act_is_null_when_no_season_resolves() {
        // Nothing to label the peak with when the season list can't place the act.
        let players = vec![base_player("me", "Blue")];
        let case = Case { own_team: Some("Blue".into()), ..Default::default() };
        let rows = case.run(&players, "me");
        assert_eq!(row(&rows, "me").peak_rank_act, None);
    }

    #[test]
    fn zero_wire_level_is_hidden_not_a_real_level() {
        // Riot zeroes AccountLevel when the level is hidden — even on your own row.
        let mut me = base_player("me", "Blue");
        me.account_level = 0;
        let players = vec![me];
        let case = Case { own_team: Some("Blue".into()), ..Default::default() };
        let rows = case.run(&players, "me");
        assert_eq!(row(&rows, "me").account_level, None);
    }

    #[test]
    fn missing_mmr_is_unranked_not_error() {
        let players = vec![base_player("me", "Blue")];
        let case = Case { own_team: Some("Blue".into()), ..Default::default() };
        let rows = case.run(&players, "me");
        assert_eq!(row(&rows, "me").current_rank.tier, 0);
        assert_eq!(row(&rows, "me").current_rank.name, "Unranked");
        assert!(row(&rows, "me").win_rate.is_none());
    }

    #[test]
    fn party_id_only_set_for_party_members() {
        let players = vec![base_player("me", "Blue"), base_player("solo", "Blue")];
        let mut parties = HashMap::new();
        parties.insert("me".to_string(), "party".to_string());
        let case = Case { parties, own_team: Some("Blue".into()), ..Default::default() };
        let rows = case.run(&players, "me");
        assert_eq!(row(&rows, "me").party_id.as_deref(), Some("party"));
        assert_eq!(row(&rows, "solo").party_id, None);
    }

    #[test]
    fn wires_phase2_stats_onto_rows() {
        let players = vec![base_player("me", "Blue")];
        let mut updates = HashMap::new();
        updates.insert(
            "me".to_string(),
            RrHistory {
                rr_change: Some(13),
                results: vec![crate::riot::types::MatchResult::Win, crate::riot::types::MatchResult::Loss],
                recent_match_ids: vec!["m1".into()],
            },
        );
        let mut recent_stats = HashMap::new();
        recent_stats
            .insert("me".to_string(), RecentStats { headshot_percent: Some(25), kd: Some(1.28) });
        let mut skins = HashMap::new();
        skins.insert(
            "me".to_string(),
            PlayerSkinIds {
                vandal: EquippedSkin { skin: Some("van".into()), chroma: Some("van-c2".into()) },
                phantom: EquippedSkin::default(),
            },
        );
        let case =
            Case { updates, recent_stats, skins, own_team: Some("Blue".into()), ..Default::default() };
        let rows = case.run(&players, "me");
        let me = row(&rows, "me");
        assert_eq!(me.rr_change, Some(13));
        assert_eq!(me.recent_results.len(), 2);
        assert_eq!(me.headshot_percent, Some(25));
        assert_eq!(me.kd, Some(1.28));
        // Skin id present but static data empty -> resolves to an (empty-name) SkinInfo.
        assert!(me.vandal_skin.is_some());
        assert!(me.phantom_skin.is_none());
    }

    #[test]
    fn missing_recent_stats_leave_hs_and_kd_null() {
        // Phase-1 rows (nothing gathered yet) and players with no recent matches both land
        // here: no entry in the map -> both figures null, no other field affected.
        let players = vec![base_player("me", "Blue")];
        let case = Case { own_team: Some("Blue".into()), ..Default::default() };
        let rows = case.run(&players, "me");
        let me = row(&rows, "me");
        assert_eq!(me.headshot_percent, None);
        assert_eq!(me.kd, None);
    }

    #[test]
    fn an_absent_group_is_pending_until_the_build_settles() {
        // Mid-build the row still renders its unranked shapes, but says the numbers are
        // coming.
        let players = vec![base_player("me", "Blue")];
        let case = Case {
            own_team: Some("Blue".into()),
            ..Default::default()
        };
        let rows = case.run(&players, "me");
        let me = row(&rows, "me");
        assert!(me.pending.rank);
        assert_eq!(me.current_rank.tier, 0);
        assert_eq!(me.current_rank.name, "Unranked");
        assert!(me.pending.name);
        assert!(me.pending.history);
        assert!(me.pending.recent_stats);
    }

    #[test]
    fn the_settled_build_has_no_pending_flags() {
        // Once the match is final an absent value is genuinely absent, never a skeleton.
        let players = vec![base_player("me", "Blue")];
        let case = Case {
            own_team: Some("Blue".into()),
            finality: true,
            ingame: true,
            ..Default::default()
        };
        let rows = case.run(&players, "me");
        assert_eq!(
            row(&rows, "me").pending,
            crate::riot::types::PendingStats::default()
        );
    }

    #[test]
    fn a_fetched_group_stops_being_pending_mid_build() {
        let players = vec![base_player("me", "Blue")];
        let mut names = HashMap::new();
        names.insert("me".to_string(), "Me#1".to_string());
        let mmr = mmr_map(&[("me", json!({}))]);
        let mut recent_stats = HashMap::new();
        recent_stats.insert("me".to_string(), RecentStats::default());
        let case = Case {
            names,
            mmr,
            recent_stats,
            own_team: Some("Blue".into()),
            ..Default::default()
        };
        let rows = case.run(&players, "me");
        let me = row(&rows, "me");
        assert!(!me.pending.name);
        assert!(!me.pending.rank);
        assert!(!me.pending.recent_stats);
        // The two match-history groups settle independently.
        assert!(me.pending.history);
    }

    #[test]
    fn skins_are_pending_only_while_an_ingame_loadout_is_outstanding() {
        let players = vec![base_player("me", "Blue")];
        let pending_skins = |ingame, loadouts_fetched| {
            let case = Case {
                own_team: Some("Blue".into()),
                ingame,
                loadouts_fetched,
                ..Default::default()
            };
            case.run(&players, "me")[0].pending.skins
        };
        assert!(pending_skins(true, false));
        assert!(!pending_skins(true, true)); // loadouts settle for the whole roster at once
        assert!(!pending_skins(false, false)); // pregame has no loadouts to wait for
    }

    #[test]
    fn orders_ally_block_first_self_first_then_by_name() {
        // Enemies and allies interleaved on input; own is "me" on Blue.
        let players = vec![
            base_player("enemy-zed", "Red"),
            base_player("ally-bob", "Blue"),
            base_player("me", "Blue"),
            base_player("enemy-amy", "Red"),
            base_player("ally-ann", "Blue"),
            base_player("ally-hidden", "Blue"),
        ];
        let mut names = HashMap::new();
        names.insert("me".into(), "Zeta#1".into()); // self name should NOT affect self-first
        names.insert("ally-bob".into(), "Bob#1".into());
        names.insert("ally-ann".into(), "ann#1".into()); // lowercase to test case-insensitive
        names.insert("enemy-zed".into(), "Zed#1".into());
        names.insert("enemy-amy".into(), "Amy#1".into());
        // ally-hidden has no name (unresolved) -> sorts last within the ally block.
        let case = Case { names, own_team: Some("Blue".into()), ..Default::default() };
        let rows = case.run(&players, "me");
        let order: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            order,
            vec![
                "me",          // self first (ally block)
                "ally-ann",    // then allies by name: ann < bob
                "ally-bob",
                "ally-hidden", // unresolved name last in ally block
                "enemy-amy",   // enemy block, by name: amy < zed
                "enemy-zed",
            ]
        );
        // All allies precede all enemies.
        assert!(rows.iter().take(4).all(|r| r.is_ally));
        assert!(rows.iter().skip(4).all(|r| !r.is_ally));
    }

    #[test]
    fn ordering_is_deterministic_tiebreak_by_puuid() {
        // Two allies with identical (None) names -> tiebreak by puuid, stable.
        let players = vec![
            base_player("b-ally", "Blue"),
            base_player("a-ally", "Blue"),
            base_player("me", "Blue"),
        ];
        let case = Case { own_team: Some("Blue".into()), ..Default::default() };
        let rows = case.run(&players, "me");
        let order: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        // self first, then puuid order a-ally < b-ally.
        assert_eq!(order, vec!["me", "a-ally", "b-ally"]);
    }
}
