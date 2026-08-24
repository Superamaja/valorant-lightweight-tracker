//! Assemble display-ready `PlayerRow`s from the fetched match/name/rank data.
//! Pure and fixture-testable — this is where privacy rules (incognito, HideAccountLevel)
//! and rank resolution are applied. See spec §5, §7, §8.

use crate::riot::match_state::MatchPlayer;
use crate::riot::rank::{self, MmrResponse};
use crate::riot::static_data::StaticData;
use crate::riot::types::PlayerRow;
use std::collections::HashMap;

/// All the resolved inputs needed to build the table.
pub struct AssembleInput<'a> {
    pub players: &'a [MatchPlayer],
    /// puuid -> "GameName#TagLine".
    pub names: &'a HashMap<String, String>,
    /// puuid -> parsed MMR (missing entries -> Unranked).
    pub mmr: &'a HashMap<String, MmrResponse>,
    /// puuid -> partyId (only real parties, size > 1).
    pub parties: &'a HashMap<String, String>,
    pub static_data: &'a StaticData,
    pub own_puuid: &'a str,
    pub own_team: Option<&'a str>,
    pub current_season_id: &'a str,
}

/// Build the player rows. Applies incognito/level privacy rules and rank resolution.
pub fn assemble_players(input: &AssembleInput) -> Vec<PlayerRow> {
    let own_party = input.parties.get(input.own_puuid).cloned();
    let default_mmr = MmrResponse::default();

    input
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

            // Account level: shown to self + own party regardless; otherwise gated by flag.
            let level_visible = !p.hide_account_level || is_self || is_party_of_self;
            let account_level = if level_visible { Some(p.account_level) } else { None };

            // Ranks.
            let mmr = input.mmr.get(&p.puuid).unwrap_or(&default_mmr);
            let current = rank::compute_current(mmr, input.current_season_id);
            let peak = rank::compute_peak(mmr, current.tier);

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
                account_level,
                party_id: player_party,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn marks_self_and_ally() {
        let players = vec![base_player("me", "Blue"), base_player("foe", "Red")];
        let names = HashMap::new();
        let mmr = HashMap::new();
        let parties = HashMap::new();
        let sd = StaticData::default();
        let rows = assemble_players(&AssembleInput {
            players: &players,
            names: &names,
            mmr: &mmr,
            parties: &parties,
            static_data: &sd,
            own_puuid: "me",
            own_team: Some("Blue"),
            current_season_id: "s1",
        });
        assert!(rows[0].is_self && rows[0].is_ally);
        assert!(!rows[1].is_self && !rows[1].is_ally);
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
        let rows = assemble_players(&AssembleInput {
            players: &players,
            names: &names,
            mmr: &HashMap::new(),
            parties: &HashMap::new(),
            static_data: &StaticData::default(),
            own_puuid: "me",
            own_team: Some("Blue"),
            current_season_id: "s1",
        });
        assert_eq!(rows[0].name.as_deref(), Some("Me#1")); // self visible
        assert_eq!(rows[1].name, None); // incognito enemy hidden
        assert!(rows[1].incognito);
    }

    #[test]
    fn hide_account_level_gated_by_party_and_self() {
        let mut foe = base_player("foe", "Red");
        foe.hide_account_level = true;
        let mut mate = base_player("mate", "Blue");
        mate.hide_account_level = true;
        let players = vec![foe, mate];
        // me and mate share a party.
        let mut parties = HashMap::new();
        parties.insert("me".to_string(), "party".to_string());
        parties.insert("mate".to_string(), "party".to_string());
        let rows = assemble_players(&AssembleInput {
            players: &players,
            names: &HashMap::new(),
            mmr: &HashMap::new(),
            parties: &parties,
            static_data: &StaticData::default(),
            own_puuid: "me",
            own_team: Some("Blue"),
            current_season_id: "s1",
        });
        assert_eq!(rows[0].account_level, None); // enemy hiding level -> blanked
        assert_eq!(rows[1].account_level, Some(100)); // party member -> shown
    }

    #[test]
    fn resolves_current_and_peak_rank() {
        let players = vec![base_player("me", "Blue")];
        let mmr = mmr_map(&[(
            "me",
            json!({ "QueueSkills": { "competitive": { "SeasonalInfoBySeasonID": {
                "s1": { "CompetitiveTier": 13, "RankedRating": 40, "LeaderboardRank": 0,
                        "WinsByTier": { "24": 2 } }
            }}}}),
        )]);
        let rows = assemble_players(&AssembleInput {
            players: &players,
            names: &HashMap::new(),
            mmr: &mmr,
            parties: &HashMap::new(),
            static_data: &StaticData::default(),
            own_puuid: "me",
            own_team: Some("Blue"),
            current_season_id: "s1",
        });
        assert_eq!(rows[0].current_rank.tier, 13);
        assert_eq!(rows[0].current_rank.name, "Gold 2");
        assert_eq!(rows[0].rr, 40);
        assert_eq!(rows[0].peak_rank.tier, 24); // Immortal 1 from WinsByTier
        assert_eq!(rows[0].peak_rank.name, "Immortal 1");
    }

    #[test]
    fn missing_mmr_is_unranked_not_error() {
        let players = vec![base_player("me", "Blue")];
        let rows = assemble_players(&AssembleInput {
            players: &players,
            names: &HashMap::new(),
            mmr: &HashMap::new(),
            parties: &HashMap::new(),
            static_data: &StaticData::default(),
            own_puuid: "me",
            own_team: Some("Blue"),
            current_season_id: "s1",
        });
        assert_eq!(rows[0].current_rank.tier, 0);
        assert_eq!(rows[0].current_rank.name, "Unranked");
    }

    #[test]
    fn party_id_only_set_for_party_members() {
        let players = vec![base_player("me", "Blue"), base_player("solo", "Blue")];
        let mut parties = HashMap::new();
        parties.insert("me".to_string(), "party".to_string());
        let rows = assemble_players(&AssembleInput {
            players: &players,
            names: &HashMap::new(),
            mmr: &HashMap::new(),
            parties: &parties,
            static_data: &StaticData::default(),
            own_puuid: "me",
            own_team: Some("Blue"),
            current_season_id: "s1",
        });
        assert_eq!(rows[0].party_id.as_deref(), Some("party"));
        assert_eq!(rows[1].party_id, None);
    }
}
