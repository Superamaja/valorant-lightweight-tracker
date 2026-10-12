# Data Sources

## Riot local client API (primary live data)

- Valorant's client runs a local HTTPS server. Credentials come from the **lockfile**: `%LOCALAPPDATA%\Riot Games\Riot Client\Config\lockfile` (format: `name:pid:port:password:protocol`). Auth is HTTP basic (`riot:<password>`), cert is self-signed — the HTTP client must accept it.
- From local endpoints you get session/user info and entitlement + access tokens.
- Those tokens authorize calls to Riot's **remote** endpoints (`pd.<shard>.a.pvp.net`, `glz-<region>-1.<shard>.a.pvp.net`) for match info: current game/pregame, player MMR, competitive updates (rank, RR, peak rank).
- A local **websocket** (same port/auth) pushes presence/state events — how vRY detects menu → pregame → ingame.

## Game server labels

The header's server label comes from the match `GamePodID` (city token mapped in
`src-tauri/src/riot/constants.rs`, `game_server_label`). Labels are the in-game server picker's,
per Riot Support "Server Select":
https://support-valorant.riotgames.com/hc/en-us/articles/360055678634-Server-Select

- **Confirmed pod tokens** (seen in real pod ids): all six NA tokens `norcal`, `oregon`,
  `ashburn`, `dallas`, `chicago`, `atlanta` (live `pods.json` capture on the NA shard,
  2026-10-12: `aresriot.aws-atl2-prod.na-gp-atlanta-2`, `aresriot.aws-chi2-prod.na-gp-chicago-2`,
  `aresriot.aws-dfw2-prod.na-gp-dallas-2`, `aresriot.aws-use1-prod.na-gp-ashburn-1`,
  `aresriot.aws-usw1-prod.na-gp-norcal-1`, `aresriot.aws-usw2-prod.na-gp-oregon-1`), plus
  `hongkong`, `tokyo`, `singapore`, `sydney`, `mumbai`. The trailing instance number is not
  always `1` (`-2` occurs); the parser accepts any digits.
- **Inferred tokens** (label from Riot, token guessed): `santiago`, `mexicocity`, `miami`, `saopaulo`, `frankfurt`, `paris`, `stockholm`,
  `istanbul`, `london`, `warsaw`, `madrid`, `bahrain`, `seoul`, `manila`. An unmatched token shows
  title-cased, so a wrong guess degrades to a readable city name. Confirm them from the
  `GamePodPingServiceInfo` keys in `pnpm snapshot`'s `pods.json` (one capture per region).

## Reference material

- vRY source (correctness reference): https://github.com/mdevio/VALORANT-rank-yoinker — see `src/requestsV.py` (endpoint plumbing), `src/presences.py`, `src/rank.py`, `src/player_stats.py`. ISC license, so borrowing logic is fine.
- Community endpoint docs (techchrism): https://valapidocs.techchrism.me/
- `valclient.py` (PyPI) — maintained Python wrapper over the same endpoints; relevant if the Python stack is chosen.

## valorant-api.com (static data)

- Free, no API key. Agents, ranks (tier icons/names), weapon skins, maps, game modes — all imagery a good UI needs.
- Data is versioned per game patch; cache it locally and refresh when the game version changes.
- `/v1/agents` is fetched unfiltered: non-playable entries matter, e.g. Gauntlet: Glitched's draft agent `773f0c78-4486-752b-68ef-4585d7f4b848` ("KAY/O", developerName `AbilityDraftAgent`, `isPlayableCharacter: false`), which every player in that mode carries as `CharacterID`.

## Warnings

- ValoTracker (https://github.com/Londopy/ValoTracker): do NOT copy its backend logic — its displayed data is wrong (user-verified).
- Unofficial API surface: Riot can change endpoints on any patch. When data breaks, diff vRY's recent commits first — they usually fix breakage quickly.
