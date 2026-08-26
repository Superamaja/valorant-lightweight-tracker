//! Static tables ported verbatim from vRY `constants.py`
//! (commit 0e30d916d366ecff6433ff6e95f69fee93a3608c) plus a couple of
//! request-header constants. These are historical / rarely-changing values.

/// Tier number (`CompetitiveTier`) -> rank name. Index == tier number, 28 entries.
/// Source: vRY `NUMBERTORANKS` (color codes stripped — irrelevant to us).
pub const NUMBER_TO_RANK: [&str; 28] = [
    "Unranked",     // 0
    "Unranked",     // 1
    "Unranked",     // 2
    "Iron 1",       // 3
    "Iron 2",       // 4
    "Iron 3",       // 5
    "Bronze 1",     // 6
    "Bronze 2",     // 7
    "Bronze 3",     // 8
    "Silver 1",     // 9
    "Silver 2",     // 10
    "Silver 3",     // 11
    "Gold 1",       // 12
    "Gold 2",       // 13
    "Gold 3",       // 14
    "Platinum 1",   // 15
    "Platinum 2",   // 16
    "Platinum 3",   // 17
    "Diamond 1",    // 18
    "Diamond 2",    // 19
    "Diamond 3",    // 20
    "Ascendant 1",  // 21
    "Ascendant 2",  // 22
    "Ascendant 3",  // 23
    "Immortal 1",   // 24
    "Immortal 2",   // 25
    "Immortal 3",   // 26
    "Radiant",      // 27
];

/// Season/act UUIDs predating the Ascendant tier (Episode 3 Act 1 / patch 4.0).
/// Old-season Immortal/Radiant `WinsByTier` keys > 20 must be shifted +3 so they
/// do not collide with the modern Ascendant range. Frozen historical list — never grows.
/// Source: vRY `before_ascendant_seasons` (17 entries).
pub const BEFORE_ASCENDANT_SEASONS: [&str; 17] = [
    "0df5adb9-4dcb-6899-1306-3e9860661dd3",
    "3f61c772-4560-cd3f-5d3f-a7ab5abda6b3",
    "0530b9c4-4980-f2ee-df5d-09864cd00542",
    "46ea6166-4573-1128-9cea-60a15640059b",
    "fcf2c8f4-4324-e50b-2e23-718e4a3ab046",
    "97b6e739-44cc-ffa7-49ad-398ba502ceb0",
    "ab57ef51-4e59-da91-cc8d-51a5a2b9b8ff",
    "52e9749a-429b-7060-99fe-4595426a0cf7",
    "71c81c67-4fae-ceb1-844c-aab2bb8710fa",
    "2a27e5d2-4d30-c9e2-b15a-93b8909a442c",
    "4cb622e1-4244-6da3-7276-8daaf1c01be2",
    "a16955a5-4ad0-f761-5e9e-389df1c892fb",
    "97b39124-46ce-8b55-8fd1-7cbf7ffe173f",
    "573f53ac-41a5-3a7d-d9ce-d6a6298e5704",
    "d929bc38-4ab6-7da4-94f0-ee84f8ac141e",
    "3e47230a-463c-a301-eb7d-67bb60357d4f",
    "808202d6-4f2b-a8ff-1feb-b3a0590ad79f",
];

/// queueId -> display mode name. Source: vRY `gamemodes`.
/// An empty queueId maps to "Custom" (customs report an empty queue).
pub fn game_mode_name(queue_id: &str) -> String {
    let name = match queue_id {
        "newmap" => "New Map",
        "competitive" => "Competitive",
        "unrated" => "Unrated",
        "swiftplay" => "Swiftplay",
        "spikerush" => "Spike Rush",
        "deathmatch" => "Deathmatch",
        "ggteam" => "Escalation",
        "onefa" => "Replication",
        "hurm" => "Team Deathmatch",
        "custom" => "Custom",
        "snowball" => "Snowball Fight",
        "valaram" => "All Random One Site",
        "dodgeball" => "Knockout",
        "" => "Custom",
        other => return other.to_string(),
    };
    name.to_string()
}

/// Static `X-Riot-ClientPlatform` base64 blob (vRY hardcodes this; accepted regardless
/// of the real OS version). Decodes to the platformType/OS/OSVersion/Chipset JSON.
pub const CLIENT_PLATFORM: &str = "ew0KCSJwbGF0Zm9ybVR5cGUiOiAiUEMiLA0KCSJwbGF0Zm9ybU9TIjogIldpbmRvd3MiLA0KCSJwbGF0Zm9ybU9TVmVyc2lvbiI6ICIxMC4wLjE5MDQyLjEuMjU2LjY0Yml0IiwNCgkicGxhdGZvcm1DaGlwc2V0IjogIlVua25vd24iDQp9";

/// Static User-Agent vRY sends for remote calls.
pub const USER_AGENT: &str = "ShooterGame/13 Windows/10.0.19043.1.256.64bit";

// --- Phase 2: per-player stats ---------------------------------------------

/// Weapon item uuid for the Vandal in a coregame loadout's `Loadout.Items` map.
/// (Verified live — spec Live verification round 2.)
pub const VANDAL_WEAPON_ID: &str = "9c82e19d-4575-0200-1a81-3eacf00cf872";

/// Weapon item uuid for the Phantom (from valorant-api `/v1/weapons`).
pub const PHANTOM_WEAPON_ID: &str = "ee8e8d15-496b-07ac-e5f6-8fae5d4c7b1a";

/// Socket uuid whose `Item.ID` holds the equipped skin uuid, inside a weapon item's
/// `Sockets` map. (Verified live — spec Live verification round 2.)
pub const SKIN_SOCKET_ID: &str = "bcef87d6-209b-46c6-8b19-fbe40bd95abc";

/// Socket uuid whose `Item.ID` holds the equipped chroma (skin colourway) uuid. Community-
/// standard value; not yet confirmed against a live capture, and a miss degrades to the base
/// skin art rather than failing.
pub const CHROMA_SOCKET_ID: &str = "3ad1b2b2-acdb-4524-852f-954a76ddae0a";

/// Number of recent competitive matches HS% is averaged over (vRY uses 1; we widen the
/// sample for a steadier figure). match-details are ~500 KB each, so keep this small.
pub const RECENT_MATCHES_FOR_HS: usize = 5;

/// Number of W/L pips shown for the "last games" column.
pub const RECENT_RESULTS_COUNT: usize = 5;

/// `endIndex` for the competitiveupdates request (a small window covering ΔRR + pips +
/// the match ids HS% reuses). Matches the live probe.
pub const COMPETITIVE_UPDATES_END_INDEX: u32 = 10;

/// Small delay between the burst of per-player stat requests at match start, to stay
/// under Riot's rate limit alongside the existing 429 retry. Milliseconds.
pub const INTER_REQUEST_DELAY_MS: u64 = 120;

/// Map region -> pd/glz shard. shard == region except latam/br -> na; pbe -> na.
/// (Inferred from valclient.py + vRY's pbe special-case; see spec gaps.)
pub fn region_to_shard(region: &str) -> &str {
    match region {
        "latam" | "br" => "na",
        "pbe" => "na",
        other => other,
    }
}

/// pbe is force-mapped to region `na` for host construction.
pub fn normalize_region(region: &str) -> &str {
    match region {
        "pbe" => "na",
        other => other,
    }
}
