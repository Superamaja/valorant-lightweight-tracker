# Data Sources

## Riot local client API (primary live data)

- Valorant's client runs a local HTTPS server. Credentials come from the **lockfile**: `%LOCALAPPDATA%\Riot Games\Riot Client\Config\lockfile` (format: `name:pid:port:password:protocol`). Auth is HTTP basic (`riot:<password>`), cert is self-signed — the HTTP client must accept it.
- From local endpoints you get session/user info and entitlement + access tokens.
- Those tokens authorize calls to Riot's **remote** endpoints (`pd.<shard>.a.pvp.net`, `glz-<region>-1.<shard>.a.pvp.net`) for match info: current game/pregame, player MMR, competitive updates (rank, RR, peak rank).
- A local **websocket** (same port/auth) pushes presence/state events — how vRY detects menu → pregame → ingame.

## Reference material

- vRY source (correctness reference): https://github.com/mdevio/VALORANT-rank-yoinker — see `src/requestsV.py` (endpoint plumbing), `src/presences.py`, `src/rank.py`, `src/player_stats.py`. ISC license, so borrowing logic is fine.
- Community endpoint docs (techchrism): https://valapidocs.techchrism.me/
- `valclient.py` (PyPI) — maintained Python wrapper over the same endpoints; relevant if the Python stack is chosen.

## valorant-api.com (static data)

- Free, no API key. Agents, ranks (tier icons/names), weapon skins, maps, game modes — all imagery a good UI needs.
- Data is versioned per game patch; cache it locally and refresh when the game version changes.

## Warnings

- ValoTracker (https://github.com/Londopy/ValoTracker): do NOT copy its backend logic — its displayed data is wrong (user-verified).
- Unofficial API surface: Riot can change endpoints on any patch. When data breaks, diff vRY's recent commits first — they usually fix breakage quickly.
