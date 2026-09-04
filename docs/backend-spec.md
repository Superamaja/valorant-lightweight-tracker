# Backend Spec — In-Match Player Table

Last updated: 2026-08-24.

**Pinned correctness reference:** [mdevio/VALORANT-rank-yoinker](https://github.com/mdevio/VALORANT-rank-yoinker) (vRY), commit `0e30d916d366ecff6433ff6e95f69fee93a3608c` (main, 2026-08-21, "feat: add UI settings and improve match state handling"). All field paths and endpoint URLs below were read directly from vRY source at this commit. Cross-checked against techchrism's community API docs (https://valapidocs.techchrism.me/, backing repo `techchrism/valorant-api-docs`, `trunk` branch).

This is the implementation contract. Read it instead of re-reading vRY. Where vRY does something questionable purely for TUI/console reasons (color codes, `rich` table rendering, chat printing), that's called out and excluded — we only need the data pipeline, not vRY's presentation layer.

---

## 1. Lockfile and local API auth

Path: `%LOCALAPPDATA%\Riot Games\Riot Client\Config\lockfile`

Format: single line, colon-separated: `name:pid:port:password:protocol`

```
Riot Client:23144:52995:Ss4WWtBoLIdaOoYm1FLKGw:https
```

Parse into `{name, PID, port, password, protocol}`.

**Local API auth**: HTTP Basic, username `riot`, password = lockfile password.

```
Authorization: Basic <base64("riot:" + password)>
```

Base URL: `https://127.0.0.1:{port}`. Local endpoints use a **self-signed cert** — the HTTP client must skip verification (vRY uses `requests(..., verify=False)`; in Rust with `reqwest`, `danger_accept_invalid_certs(true)` on a client scoped only to `127.0.0.1`).

Retry behavior vRY uses for local calls (source: `requestsV.py` `fetch(url_type="local", ...)`): up to 3 retries, 5s sleep between, treating a non-200 or `{"errorCode": "RPC_ERROR"}` body as "not ready yet" — this happens while the Riot Client is starting up. Also handle: lockfile file not existing yet (client not running at all) — poll for the file to appear.

## 2. Token acquisition and region/shard discovery

**Entitlements + access token** (local endpoint):

```
GET https://127.0.0.1:{port}/entitlements/v1/token
Authorization: Basic <base64("riot:" + password)>
```

Response:
```ts
{
  accessToken: string,   // -> Authorization: Bearer <accessToken> for remote calls
  token: string,          // -> X-Riot-Entitlements-JWT header
  subject: string,        // -> the player's own PUUID
  issuer: string,
  entitlements: unknown[]
}
```

vRY polls this until `message` is not `"Entitlements token is not ready yet"` / `"Invalid URI format"` (client still starting). Cache the resulting headers; on any remote call returning `errorCode: "BAD_CLAIMS"`, or a non-OK response, clear the cached headers and refetch (token expired/rotated).

**Region/shard discovery** — vRY parses `%LOCALAPPDATA%\VALORANT\Saved\Logs\ShooterGame.log`, scanning lines for `.a.pvp.net/account-xp/v1/` (gives the pd shard) and `https://glz-` (gives glz region+shard). This is fragile (depends on game having logged in this session, log rotates, requires the *Valorant* game log not just Riot Client).

**Recommended alternative for our implementation** (community-documented, used by `valclient.py` too): local endpoint

```
GET https://127.0.0.1:{port}/riotclient/region-locale
Authorization: Basic <base64("riot:" + password)>
```
Response: `{ locale, region, webLanguage, webRegion }` — `region` is one of `na, eu, ap, kr, latam, br` (and `pbe`).

Shard != region for two regions. Known static mapping (matches vRY's own `pbe -> na` special-case in `requestsV.get_region`, and matches valclient.py's long-standing table):
- shard = region, **except** `latam` and `br` → shard = `na`
- `pbe` → region `na`, shard `na`

Host construction (from `requestsV.py`):
```
pd_url  = f"https://pd.{shard}.a.pvp.net"
glz_url = f"https://glz-{region}-1.{shard}.a.pvp.net"
```
e.g. region `eu`, shard `eu` → `pd.eu.a.pvp.net`, `glz-eu-1.eu.a.pvp.net`. Region `br` → shard `na` → `pd.na.a.pvp.net`, `glz-br-1.na.a.pvp.net`.

**Gap**: I could not fully verify the `latam`/`br` → `na` shard override against current vRY source (vRY derives it empirically from the log, so it never hardcodes the table) — this mapping is corroborated by `valclient.py` and long-standing community knowledge, but if a br/latam user ever reports wrong data, verify against `/riotclient/region-locale` + a captured `pd`/`glz` request in the log.

## 3. Client version and client platform headers

vRY reads the version from the same `ShooterGame.log`, line containing `CI server version: `, e.g. `CI server version: release-08.10-shipping-32-2965155-Shipping`, strips a trailing `-Shipping`-style suffix, uses the rest as-is for the header.

**Recommended alternative**: `https://valorant-api.com/v1/version` (no auth, no local dependency) returns:
```json
{"data": {"riotClientVersion": "release-13.04-shipping-18-5304478", "version": "13.04.00.5304478", ...}}
```
`riotClientVersion` is in the same format vRY sends. This avoids depending on the Valorant game log file at all — fetch once at startup and cache, refresh periodically (e.g. daily) since it changes every patch.

Full remote-call header set (from `requestsV.get_headers`):
```
Authorization: Bearer <accessToken>
X-Riot-Entitlements-JWT: <token>
X-Riot-ClientPlatform: ew0KCSJwbGF0Zm9ybVR5cGUiOiAiUEMiLA0KCSJwbGF0Zm9ybU9TIjog\
IldpbmRvd3MiLA0KCSJwbGF0Zm9ybU9TVmVyc2lvbiI6ICIxMC4wLjE5\
MDQyLjEuMjU2LjY0Yml0IiwNCgkicGxhdGZvcm1DaGlwc2V0IjogIlVua25vd24iDQp9
X-Riot-ClientVersion: <client version string, see above>
User-Agent: ShooterGame/13 Windows/10.0.19043.1.256.64bit
```
The `X-Riot-ClientPlatform` value is a static base64 blob — it decodes to:
```json
{
    "platformType": "PC",
    "platformOS": "Windows",
    "platformOSVersion": "10.0.19042.1.256.64bit",
    "platformChipset": "Unknown"
}
```
Use this exact blob verbatim (it's what vRY hardcodes; it's accepted regardless of the real OS version).

## 4. Game-state detection

Two mechanisms, used together in vRY:

**A. Local websocket** (primary, for live state-change push):
```
wss://127.0.0.1:{port}
Authorization: Basic <base64("riot:" + password)>
```
Self-signed cert, same as local HTTPS — accept it. On connect, subscribe by sending:
```
[5, "OnJsonApiEvent_chat_v4_presences"]
```
(vRY optionally also sends `[5, "OnJsonApiEvent_chat_v6_messages"]` for chat — not needed for the player table.)

Each pushed message is a JSON array; the payload of interest is at index 2:
```
msg[2].uri == "/chat/v4/presences"
msg[2].data.presences[0]   // a presence object, same shape as the REST presences list below
```
Filter for `presence.puuid == <own puuid>` (ignore other players' presence events pushed on the same subscription — you only need your own to detect state transitions). Skip if `presence.product == "league_of_legends"` (League client running alongside).

vRY reconnects with exponential backoff (5 attempts, 2s → 4s → 8s...) on `ConnectionClosed`/`InvalidURI`/refused/OS errors, and treats persistent failure as a `DISCONNECTED` app state (shown as "Attempting to reconnect...", falls back to polling the lockfile + presence REST endpoint until the client responds again).

**B. Presence REST polling** (used for the *initial* state on startup, and as the data source once a state-change event fires — the websocket only tells you *that* something changed, you then re-fetch full presence/match data via REST):
```
GET https://127.0.0.1:{port}/chat/v4/presences
Authorization: Basic <base64("riot:" + password)>
```
Response: `{ presences: [ {puuid, product, private, ...}, ... ] }` — one entry per friend/self currently online. Find your own entry (`puuid == own puuid`).

**Decoding the private presence** — `presence.private` is base64 of a JSON blob:
```
decoded = JSON.parse(base64_decode(presence.private))
```
Empty string `private` → not yet initialized, treat as "no presence yet" (poll again).

vRY notes Riot has been swapping between two response shapes and defensively checks both (**must implement both** — this is an active/recent breakage pattern, not hypothetical):
- **Nested**: `decoded.matchPresenceData.sessionLoopState`
- **Flat**: `decoded.sessionLoopState`

`sessionLoopState` is one of: `"MENUS"`, `"PREGAME"`, `"INGAME"`. This is the sole state signal — no separate "spectating" state (see Pitfalls, §10).

Also read from the same decoded private presence (nested vs flat, same pattern):
- Party state: `.partyPresenceData.partyState` / `.partyState` — value `"CUSTOM_GAME_SETUP"` combined with `presence.provisioningFlow == "CustomGame"` (top-level on the decoded object, not nested under either variant, per vRY) identifies a custom game (see §10).
- Queue id: `.queueId` (top-level) — maps to a game mode name (`competitive`, `unrated`, `swiftplay`, `spikerush`, `deathmatch`, `custom`, etc. — see `constants.py gamemodes` dict).
- Account level (self, for party display): `.playerPresenceData.accountLevel` / `.accountLevel`.
- Party id/size (for grouping party members in the table): `.partyPresenceData.{partyId,partySize}` / `.{partyId,partySize}` — every online player (not just match players) carries this in their own presence, so party membership is cross-referenced by decoding **every** presence's `private` field and grouping by matching `partyId` where `partySize > 1`.

## 5. Per-state data flow

### INGAME (priority)

1. Get match id:
```
GET {glz_url}/core-game/v1/players/{own_puuid}
```
Bearer/entitlements/etc headers (§3). Response `{"MatchID": "..."}`. `errorCode: "RESOURCE_NOT_FOUND"` means not actually in a core-game match (race between websocket event and match creation) — retry after a short delay.

2. Get match/player data:
```
GET {glz_url}/core-game/v1/matches/{match_id}
```
Response fields used:
```
MapID: string                    // e.g. "/Game/Maps/Ascent/Ascent" — lowercase + look up against valorant-api maps (§7-adjacent, see mapUrl matching in content.py)
GamePodID: string                // server identifier
Players: [
  {
    Subject: string,             // puuid
    TeamID: string,              // "Red" | "Blue"
    CharacterID: string,         // agent uuid (lowercase it before matching valorant-api agent dict)
    PlayerIdentity: {
      AccountLevel: number,
      Incognito: bool,           // streamer/incognito mode flag for this player
      HideAccountLevel: bool     // this player has opted to hide their level from others
    }
  }, ...
]
```
Own team = the `TeamID` where `Subject == own_puuid`.

### PREGAME (agent select)

1. Get match id:
```
GET {glz_url}/pregame/v1/players/{own_puuid}
```
Response `{"MatchID": "..."}`, same `RESOURCE_NOT_FOUND` handling.

2. Get match/player data:
```
GET {glz_url}/pregame/v1/matches/{match_id}
```
Response fields used:
```
ID: string
GamePodID: string
AllyTeam: { Players: [ {Subject, TeamID, CharacterID, CharacterSelectionState, PlayerIdentity: {AccountLevel, Incognito, HideAccountLevel}}, ... ] }
Teams: [ {TeamID, Players: [...]}, ... ]   // used only to find own TeamID reliably
```
**Important**: PREGAME only exposes `AllyTeam.Players` — enemy team roster is not visible during agent select (Riot doesn't expose it; this is not a vRY limitation). Enemy players only become visible once the match transitions to INGAME. The table for PREGAME therefore only ever shows your own 5. `CharacterSelectionState` is `"locked"` or `"selected"` (still picking) — agent id may be empty/unselected until locked.

### Common to both states

- `PlayerIdentity.Incognito` (streamer mode) is per-player and comes from the match endpoint itself, not from presence — this is the authoritative source, use it directly on each player row.
- `PlayerIdentity.HideAccountLevel` gates whether to display that player's level to *others*; vRY's rule: show the level anyway if the row is the local player or a party member of the local player, otherwise blank it out. Enforce the same rule.

## 6. Name resolution

```
PUT {pd_url}/name-service/v2/players
Body: [puuid, puuid, ...]         // batch, up to all match players in one call
```
Response:
```json
[{"Subject": "puuid", "GameName": "Foo", "TagLine": "1234", "PUUID": "..."}, ...]
```
Build `name = GameName + "#" + TagLine`. Batch-fetch once per state refresh for all players in the match (vRY: `get_multiple_names_from_puuid`). If the response contains `errorCode`, refresh the token (`get_headers(refresh=True)`) and retry once — this is the "BAD_CLAIMS"-adjacent expiry case for this endpoint specifically.

**Deferred/hidden names**: names can come back blank or Riot may withhold them for accounts with strict privacy — vRY has no special-case for this beyond falling back to `""`/`"#"` string concatenation; treat a missing `GameName`/`TagLine` as an unresolved name and render a placeholder rather than crashing.

## 7. Ranks

**Endpoint** (per player, `pd`):
```
GET {pd_url}/mmr/v1/players/{puuid}
```
Response path used:
```
QueueSkills.competitive.SeasonalInfoBySeasonID[seasonID] = {
  CompetitiveTier: number,
  RankedRating: number,
  LeaderboardRank: number,
  WinsByTier: { [tierNumber: string]: number } | null,
  NumberOfWinsWithPlacements: number,
  NumberOfGames: number
}
```
`seasonID` = current act's season id (see below). This single response contains **all seasons** the player has data for — no per-season endpoint call, all peak-rank computation is client-side over this one payload.

**Current rank / RR / leaderboard position** — logic from `rank.py get_rank`:
- `tier = SeasonalInfoBySeasonID[seasonID].CompetitiveTier`
- if `tier >= 21` (Ascendant+): show tier, RR, and `LeaderboardRank` (nonzero only for top-500-ish leaderboard players)
- elif `tier not in (0, 1, 2)` (Iron through Diamond, i.e. tier 3-20): show tier, RR; leaderboard = 0
- else (`tier` 0/1/2, unranked): tier = 0, RR = 0, leaderboard = 0
- If the player has no entry for this season at all (`KeyError`/`TypeError`), or the HTTP response isn't OK: tier = 0, RR = 0, leaderboard = 0 (render as Unranked, don't error the row)

**Peak rank** — scan **every** season in `SeasonalInfoBySeasonID`, for each season's `WinsByTier` dict, take the max key (tier number) that has `> 0` wins recorded, compare against a running max starting from current-season tier:
```
max_rank = current_tier
for season_id, season_data in SeasonalInfoBySeasonID.items():
    if season_data.WinsByTier is not None:
        for tier_key in season_data.WinsByTier:
            tier_num = int(tier_key)
            if season_id in BEFORE_ASCENDANT_SEASONS and tier_num > 20:
                tier_num += 3   # old tier numbering didn't have Ascendant; shift Immortal/Radiant up
            if tier_num > max_rank:
                max_rank = tier_num
                max_rank_season = season_id
```
`BEFORE_ASCENDANT_SEASONS` is a **hardcoded list of 17 season/act UUIDs** predating the Ascendant rank tier's introduction (Episode 3 Act 1, patch 4.0) — see `constants.py before_ascendant_seasons`. Without the +3 shift, old-season Immortal/Radiant wins would collide with the modern Ascendant tier range. **This list will never grow** (it's historical, frozen at Ascendant's launch) — copy it verbatim, no need to keep it in sync going forward.

Peak rank act/episode label: `max_rank_season` (a season UUID) is looked up in the content-service season list (§ below) to get a human act/episode number — see `content.get_act_episode_from_act_id`. This is fiddly string parsing of season `Name` fields (`"ACT III"`, `"EPISODE 7"`, or newer combined formats like `"E9A1"`) — not required for v1 if the UI only shows the peak rank icon, but needed if showing "peak: Immortal 2 (E5A3)"-style labels. Full logic is in the source; port as-is if needed, otherwise skip and only show peak tier.

**Win rate**: `NumberOfWinsWithPlacements / NumberOfGames * 100`, rounded to int; `"N/A"` if no games this season. Not required for the v1 in-match table per `ui-spec.md` — skip unless added later.

**Season id source** (content-service, `pd`-adjacent but actually `shared.<shard>` host —
identical to `region` except for `br`/`latam`, which map to shard `na`):
```
GET https://shared.{shard}.a.pvp.net/content-service/v3/content
Authorization: Bearer <accessToken>
X-Riot-Entitlements-JWT: <token>
X-Riot-ClientPlatform / X-Riot-ClientVersion / User-Agent   // same headers as pd/glz
```
Response `Seasons: [{ID, Name, Type: "act"|"episode", StartTime, EndTime, IsActive}, ...]`.
- Current season id = the entry with `Type == "act" and IsActive == true`.
- Previous season id = the `Type == "act"` entry whose `EndTime == currentSeason.StartTime`.
- Fetch this once at startup and cache; re-fetch only if the client version changes or on a new day (acts last ~2 months, no need to poll).

**Tier number → rank name mapping**: fixed 28-entry table, index = `CompetitiveTier` value:
```
0-2:  Unranked
3-5:  Iron 1/2/3
6-8:  Bronze 1/2/3
9-11: Silver 1/2/3
12-14: Gold 1/2/3
15-17: Platinum 1/2/3
18-20: Diamond 1/2/3
21-23: Ascendant 1/2/3
24-26: Immortal 1/2/3
27:    Radiant
```
(Source: `constants.py NUMBERTORANKS` — vRY's copy embeds ANSI color codes for terminal output; strip that, it's irrelevant to us.)

**Tier icons**: `https://valorant-api.com/v1/competitivetiers` (no auth). Response is an array of tier tables (one per episode-ish era, each with its own `uuid`); each has a `tiers[]` array of `{tier, tierName, smallIcon, largeIcon, ...}` where `tier` matches the `CompetitiveTier` number above. **Use the tier table matching the current game version/act** — vRY doesn't consume this endpoint at all (its UI is text-only), so this mapping is new territory for us. Take the *last* entry in the top-level array (episode tier tables are appended chronologically) as "current" and cache it alongside game version. `smallIcon`/`largeIcon` are direct PNG URLs, cache locally per `ui-spec.md`.

**Gap**: vRY has no logic for selecting which `competitivetiers` table entry is "current" (it never calls this endpoint) — the "take the last array entry" approach is inferred, not verified against vRY. Worth a quick manual sanity check against a live match (rank icon should match what the in-game client shows) once implemented.

## 8. Account level and incognito/streamer-mode handling

Both come from the match endpoint's `PlayerIdentity` object (§5), **not** presence:
- `PlayerIdentity.AccountLevel: number`
- `PlayerIdentity.Incognito: bool` — true means this player has enabled Valorant's own streamer/incognito mode; vRY's convention (and ours, per project privacy norms) is to treat this the same as a locally-configured "hide names" feature — i.e. respect the *player's own* opt-in privacy choice by not fully de-anonymizing them in UI copy, even though the raw name/tag is still technically returned by the name-service call. vRY substitutes their identity with "`<Agent> on your/enemy team`" for its local "already played with" history feature when `Incognito` is true — not required for our v1 scope (no match-history feature), but the flag should still gate anything equivalent if we add it later.
- `PlayerIdentity.HideAccountLevel: bool` — gates whether *level* specifically is shown to others; local player and party members always see it regardless (§5).

There is no separate "incognito" field in presence for this purpose — presence-level privacy only affects whether *your own* presence is visible to friends (not relevant to the in-match table, which reads the match endpoint directly).

## 9. Suggested module breakdown (Rust / Tauri 2)

Per project convention ("all interesting logic lives in one language behind an adapter interface" — `project-context.md`), **keep all of this in Rust**. The frontend should receive already-shaped, display-ready data (or close to it — icon URLs, resolved names, numeric tiers) and do zero Riot-API-shape interpretation itself.

Suggested Rust module layout (`src-tauri/src/`):

```
riot/
  lockfile.rs      // find + parse + watch the lockfile; local basic-auth header builder
  local_api.rs      // local HTTPS client (self-signed cert accepted, 127.0.0.1 only): entitlements token, presences REST, region-locale
  remote_api.rs     // pd/glz/shared HTTP clients: header assembly (§3), region->shard host construction (§2), retry/backoff, BAD_CLAIMS handling
  websocket.rs      // local wss client: subscribe, reconnect/backoff, presence event -> state transition, dispatch to app state
  presence.rs       // decode private presence (nested vs flat shape handling), party grouping
  match_state.rs    // pregame.rs + coregame.rs equivalents: match id + player list fetch for each state
  names.rs          // batch name-service resolution
  rank.rs           // mmr fetch, current/peak rank computation, before-ascendant tier shift, season lookup
  content.rs        // content-service season list, act/episode parsing (only if peak-rank act label is implemented)
  static_data.rs     // valorant-api.com fetch + local disk cache for agents/maps/competitivetiers, keyed by game version
  types.rs          // shared structs: PlayerRow, MatchState, RankInfo, etc. — the shape handed to the frontend via Tauri commands/events
app_state.rs        // orchestration: state machine (MENUS/PREGAME/INGAME/DISCONNECTED), drives the above, emits Tauri events to frontend
```

**What stays in Rust** (all of it, effectively): lockfile parsing, all HTTP/WS calls, presence decoding, region/shard/host derivation, rank math (current + peak + tier shift), name resolution, static-data caching logic.

**What's plain data to TypeScript**: a single `PlayerRow` struct/event payload per player — puuid, resolved name, team, agent id + icon URL (resolved via `static_data.rs`, not raw uuid), current tier number + icon URL + RR, peak tier number + icon URL, account level (or `null` if hidden), party group id, is-self flag. Plus a top-level `MatchState` event — app state enum, map name + splash URL, mode name, own team, last-updated timestamp. The frontend does formatting/layout/theming only — no Riot-shape knowledge, matches `ui-spec.md`'s "images over text" + websocket-driven auto-refresh requirements directly.

Emit updates to the frontend via Tauri's event system (`app.emit(...)`) on every resolved state change, rather than polling commands from the frontend — this maps directly onto vRY's own websocket-driven loop and keeps the adapter seam Discord RPC can plug into later (per `project-context.md` architecture intent) as another consumer of the same internal `MatchState`/`PlayerRow` events.

## 10. Pitfalls vRY handles that must not be missed

1. **Client not running / lockfile missing.** Poll for the lockfile file to appear; don't crash on `FileNotFoundError`. vRY has a dedicated `Error.LockfileError` check before every lockfile read.
2. **Local API "not ready yet".** Riot Client can be running before its local API is serving real data — expect `RPC_ERROR` / `"Entitlements token is not ready yet"` responses for a few seconds after launch; retry, don't treat as fatal.
3. **Token expiry mid-session (`BAD_CLAIMS`).** Any `pd`/`glz` response with `errorCode: "BAD_CLAIMS"` means the cached bearer/entitlements headers are stale — clear and refetch, then retry the same call. Also apply the equivalent refresh-and-retry to the name-service call (§6), which returns a different `errorCode` shape.
4. **Presence structure has two live shapes** (nested `matchPresenceData.sessionLoopState` vs flat `sessionLoopState`, same split for `partyPresenceData`) — Riot has been actively switching between these; vRY added defensive dual-path handling recently (this commit's own changelog: "improve match state handling"). Implement both, don't assume one is retired.
5. **404 / `RESOURCE_NOT_FOUND` on match-id endpoints is a normal race**, not an error — happens right after the websocket reports a state change but before Riot's backend has created the match record yet. Retry with a short delay (vRY: one retry after 5s for coregame, immediate single retry for pregame) rather than surfacing an error state.
6. **429 rate limiting.** vRY backs off 5-10s and clears cached headers before retrying on any 429 from pd/glz. Don't hammer on failure.
7. **Spectators.** Not specially handled by vRY — a spectating player's own presence still reports `INGAME` with a `core-game/v1/players/{puuid}` match id, and the match's `Players` list is the same full roster either way. No separate branch needed; a spectator just sees the same data a participant would. (vRY doesn't distinguish spectator vs. player rows in `Players` — there's no `isSpectator`-type field surfaced in the coregame player list at this commit.)
8. **Custom games.** Detected via presence (`provisioningFlow == "CustomGame"` or decoded `partyState == "CUSTOM_GAME_SETUP"`), used only to relabel the game mode as "Custom Game" (queueId is otherwise empty/misleading for customs) — the match-id/player-list flow is identical to any other match, no special-casing needed beyond the mode label.
9. **Streamer/incognito mode.** Respect `PlayerIdentity.Incognito` and `HideAccountLevel` per §8 — don't let cached/prior-match data leak a previously-seen real identity for a player who has since gone incognito.
10. **League of Legends running alongside.** A presence entry can be a LoL presence (`presence.championId` present, or `presence.product == "league_of_legends"`) if the user has both clients open under one Riot Client — skip/ignore that presence entry rather than trying to decode it as Valorant presence.
11. **PBE / non-standard shard.** `pbe` region log lines get force-mapped to `na`/`na-1`/`na` by vRY. Unlikely to matter for a consumer app (PBE is opt-in, separate installation) but keep the fallback rather than erroring.
12. **State-change flapping / stale match id on disconnect.** On `INGAME -> not INGAME` transition, vRY clears its tracked match id/team immediately so a subsequent reconnect doesn't reuse stale match context; on any transition *into* `MENUS`, it invalidates all cached rank/stat responses (`rank.invalidate_cached_responses()`) since a new match means potentially-changed RR for every player. Mirror both: don't let cached per-match data bleed across matches.
13. **PREGAME shows only your own team.** Don't build UI/data-model assumptions expecting 10 players during agent select — Riot's API only exposes `AllyTeam` at that stage (§5); this is a hard platform limitation, not something to "fix".
14. **Rank endpoint failure for an individual player** (private profile, API hiccup, brand-new account with no MMR record) must not fail the whole table — vRY falls back to tier 0/RR 0/Unranked for that single row (§7) rather than erroring the batch.

---

## Gaps not determined from vRY source (be aware before implementing)

- **`latam`/`br` → `na` pd-shard override**: inferred from community convention (valclient.py), not present as an explicit table in vRY (vRY derives it empirically per-user from the game log, so it has no hardcoded table to confirm against). Verify against a real br/latam account if one is available. The diagnostics report (`get_diagnostics`) prints the region-locale region, the shard it was mapped to, and a note marking the mapping as never live-verified, so a report from a br/latam user settles this without asking them for anything else.
- **Which `competitivetiers` table entry from valorant-api.com is "current"**: vRY never consumes this endpoint (its output is text-only), so there's no reference behavior to copy. The "last array entry = current act's tier table" assumption is standard among other Valorant tools but should be sanity-checked against a live match's rank icon.
- **Exact wording/availability of the `AccountXP`/other level-related fields**: not investigated — §8 covers only what vRY actually reads (`PlayerIdentity.AccountLevel`), which was sufficient for the ui-spec's "account level" row requirement.
- **Win-rate and "already played with" history features**: present in vRY but out of scope per `ui-spec.md`/`project-context.md` (v1 = in-match table only); not detailed beyond a pointer to `rank.py`/`stats.py` in case a later milestone wants them.

---

## Implementation notes

Added 2026-08-24 when the Rust backend was implemented (`src-tauri/src/riot/` + `app_state.rs`).
The pipeline follows this spec; the notes below record deviations, decisions, and items that
could not be verified because Valorant was not running on the build machine (no live client).

### Decisions / deviations from the spec text

- **Region discovery**: used the recommended `/riotclient/region-locale` local endpoint (not
  the `ShooterGame.log` scrape). No dependency on the game log at all.
- **Client version**: `valorant-api.com/v1/version` `riotClientVersion` is used only as the
  **bootstrap/fallback** value (fetched once per connect, cached with static data). Once
  connected, `build_snapshot` overrides it with the version read from own presence
  (`partyPresenceData.partyClientVersion`, dual-path nested/flat) via
  `RemoteClient::set_client_version`. This is because valorant-api.com can lag the real
  client — observed live: valorant-api reported `shipping-18` while the running client was
  `shipping-20` (see Live verification). pd accepted the stale header that day, but on patch
  days Riot may reject it, so own-presence is authoritative when available.
- **Shard/host + pbe**: `constants::region_to_shard` / `normalize_region` implement
  `latam`/`br` → `na` and `pbe` → `na`/`na`. **Unverified against a live br/latam/pbe account.**
  The `shared` content-service host follows the **shard** (not the region), matching pd/glz —
  identical to region except `br`/`latam` (shard `na`). (Corrected from an earlier
  `shared.<region>` that would have hit a nonexistent `shared.br`/`shared.latam` host.)
- **competitivetiers "current" table**: `static_data::parse_competitive_tiers` takes the
  **last** entry of the top-level `data` array (the inferred convention). **Unverified against
  a live rank icon** — flagged for a manual sanity check.
- **Presence dual-shape**: `presence::extract_info` reads every field from BOTH the nested
  (`matchPresenceData`/`partyPresenceData`/`playerPresenceData`) and flat locations via a
  `dual()` helper; `accountLevel`/`partySize` also accept numeric-string values defensively.
  Unit-tested for both shapes, but only real traffic will confirm which Riot currently serves.
- **State enum**: the frontend-facing `AppStatus` is exactly the four states requested
  (`ValorantNotRunning | Menus | Pregame | Ingame`). The spec's "DISCONNECTED / reconnecting"
  nuance is folded into `ValorantNotRunning` + a human `message` field on the snapshot, rather
  than a separate enum variant, to keep the UI contract simple. `MENUS` and any *unknown*
  `sessionLoopState` both render as `Menus` with an empty roster.
- **Incognito → name hidden**: a player with `PlayerIdentity.Incognito == true` has `name` set
  to `null` in the row (except your own row), implementing the "don't de-anonymize" rule at the
  data layer. vRY's raw behaviour still returns the name from name-service; we withhold it in
  the emitted snapshot so a hidden name can never reach the UI. Adjust if the UI would rather
  receive the name plus a flag and decide for itself.
- **Season label parsing** (`content.get_act_episode_from_act_id`): implemented in
  `content::act_label` for UI revision 2 — see the revision-2 note at the end of this section.
- **Win rate / match-history**: implemented in phase 2 (see the phase-2 section below). WR
  reuses the phase-1 MMR payload; HS% reuses the competitiveupdates match-id list instead of a
  separate match-history call.
- **Presence poke classification (revised 2026-08-25)**: only our OWN Valorant presence event
  is forwarded as a `Poke`; §4-A's "ignore other players' presence events" holds in every state,
  Pregame included. Another player's event cannot be narrowed to our lobby at the websocket (the
  roster isn't known there), so honouring them meant every online friend's presence driving a
  pregame rebuild — and, once a poke could also cancel an in-flight request, cutting real work
  short. The 1 s pregame poll below already detects everything such an event could signal, so it
  is the sole pregame change detector. A drained burst collapses to one rebuild, and the
  reconnect re-poll sends the same `Poke`.
- **Pregame poll tick (2026-08-25)**: presence events alone are not enough during agent select.
  Riot pushes no presence event when a **non-friend** lobby player picks or locks an agent, and
  our own presence doesn't change either — live captures showed 3 rebuilds in the first ~8 s of
  Pregame (the entry events) and then **zero for ~100 s** until Ingame, so teammates' picks
  never appeared. vRY sees them because its main loop polls. `app_state::wait_for_rebuild_poke`
  therefore bounds its wait with `tokio::time::timeout` whenever `poll_interval(status)` is
  `Some` — Pregame only, at `PREGAME_POLL_MS = 1000`. An elapsed tick is a rebuild trigger
  equivalent to a poke, so the pregame `CharacterID` / `agentSelectionState` per ally refreshes
  every second. Cost: **one remote glz GET per second** — `pregame/v1/matches/{id}`, the change
  detector itself (the endpoints are glz, not local; only presence and the lockfile are local) —
  **only** during agent select, since the fully-cached path serves the rest (no name/MMR/stat
  refetch). Identical snapshots are suppressed by the existing `publish` dedup, so an unchanged
  roster emits nothing. Menus, Ingame and the not-running states get `None` and stay purely
  event-driven — **there is no ingame polling**. A tick can't stack with pokes: the rebuild
  drains the channel as usual.
- **Pregame tick cost reductions (2026-08-25)**: three changes cut what agent select spends,
  without changing what it detects.
  - **The tick stops once the roster is fully locked.** `match_state::roster_fully_locked` is
    true when every player in the pregame roster has `CharacterSelectionState == "locked"` AND a
    non-empty `CharacterID`; nothing in the payload can change after that, so
    `poll_interval(status, roster_locked)` returns `None` and the loop goes back to the purely
    event-driven wait for the rest of agent select (usually its longest stretch). The
    pregame→ingame transition is caught independently by the own-presence poke, and a dodge
    likewise cancels the lobby through a presence-driven state change. Anything unclear counts
    as NOT locked and keeps the tick running: an empty roster, an absent
    `CharacterSelectionState`, a lock without an agent id. Every rebuild clears the flag before
    it fetches anything, and only a build that parses a fully locked pregame roster sets it
    again — so it is `false` for INGAME, and a build that fails earlier (404 race, unreadable
    payload) leaves the tick running instead of pausing it with nothing scheduled to lift it.
    `MatchCache::invalidate` (MENUS / not-running) clears it too.
  - **The agent-select match id is cached.** It cannot change while the lobby lasts, so
    `MatchCache::pregame_match_id` holds it and the steady-state tick spends **1 request instead
    of 2** (`pregame/v1/players` is skipped). INGAME never reads it — the coregame match-id GET
    stays the transition detector. It is dropped on any invalidation (MENUS) and on a
    `RESOURCE_NOT_FOUND` from `pregame/v1/matches/{id}`, which means the lobby is already gone,
    so the next attempt re-resolves the id and the §10.5 404-race handling applies to it again.
  - **The immediate 404 retry is deduped across backoff cycles.** §10.5's single retry (5s for
    coregame, immediate for pregame) is kept for the first cycle of a race, but
    `MatchCache::id_retry_spent` suppresses it while the same race is still unresolved, so a
    transition that outlasts several `RETRY_BACKOFF_MS` cycles costs one 404 per cycle instead
    of a back-to-back pair. The flag clears on the next match-id fetch that succeeds and on any
    invalidation. The outer backoff schedule and the poke-cuts-backoff-short behavior are
    unchanged.
- **Static-data cache**: version-keyed JSON files under
  `%LOCALAPPDATA%\valorant-lightweight-tracker\static-cache\static-<version>.json`. Image PNGs
  themselves are passed to the UI as plain valorant-api URLs (not downloaded/cached in Rust).

### Phase 2 — per-player stats (2026-08-24)

Added the five wishlist columns from `ui-spec.md` (WR, ΔRR + last-5 W/L, HS%, Vandal +
Phantom skins). New modules: `riot/stats.rs` (competitiveupdates + match-details parsing),
`riot/loadout.rs` (skin-id extraction). Extended: `riot/rank.rs` (`compute_win_rate`),
`riot/static_data.rs` (weapon-skin cache from `/v1/weapons/skins`), `riot/remote_api.rs`
(`competitive_updates`, `match_details`, `coregame_loadouts`), `riot/types.rs`
(`WinRate`, `MatchResult`, `SkinInfo` + 6 `PlayerRow` fields, later + `kd`), `app_state.rs` (stat fetch
+ caching), `riot/assemble.rs` (wiring + row ordering). New `PlayerRow` fields and exact
serde camelCase names are in `docs/ipc-contract.md`.

- **WR** — current-season `NumberOfWins / NumberOfGames` from the MMR payload already fetched
  in phase 1. **Zero new requests.** Live-verified as exactly what vRY shows (8/14 → "57
  (14)"). `null` when 0 games. (Note: this uses `NumberOfWins`, not `NumberOfWinsWithPlacements`
  — the two were equal on the probe; §7's older note said WinsWithPlacements, superseded by the
  live cross-check against the vRY row.)
- **ΔRR + last-5** — `GET /mmr/v1/players/{puuid}/competitiveupdates?startIndex=0&endIndex=10&queue=competitive`,
  1 request/player. ΔRR = `Matches[0].RankedRatingEarned`; pips from each match's
  `RankedRatingEarned` sign, **0-RR → `Unknown`** (ambiguous, per the task's edge note; vRY
  reads sign only). `AFKPenalty`/`RRPenalty` fields are present but not used for the pip.
- **HS%** — replicates vRY `player_stats._process_match_data` exactly: sum
  `headshots / (headshots+bodyshots+legshots)` across every round's `damage` entries for the
  player, `round()` to int, "N/a" when no hits. **The match id list is reused from the
  competitiveupdates response** (`Matches[].MatchID`) rather than a separate
  `match-history` request — this keeps the match-start burst to the documented budget
  (competitiveupdates already carries the ids). `RECENT_MATCHES_FOR_HS = 5` match-details per
  uncached player (vRY uses 1; widened for a steadier figure — constant in `constants.rs`).
  match-details are ~500 KB, so HS% is **cached per puuid keyed by newest competitive match
  id** in a session-lived `HsCache` (survives match→menus→match; self-invalidates when the
  player plays a new comp match). Deviation from the wishlist's "match-history + match-details"
  wording: match-history is not fetched — competitiveupdates supplies the ids for free.
- **KD (2026-08-25)** — total kills / total deaths over the **same** `RECENT_MATCHES_FOR_HS`
  window, from the **same** match-details payloads HS% already downloads: **zero new
  requests**. Source is the payload's top-level `players[]` (each entry a `subject` puuid plus
  a `stats` object with `kills`/`deaths`), accumulated in the same pass as the head/body/leg
  hits — `stats::MatchTotals` (the former `HitCounts`) now carries both, and yields the pair as
  a `stats::RecentStats { headshot_percent, kd }`. Rounded to 2 decimals; 0 deaths yields the
  kill count itself (7/0 -> 7.0); `null` when the player has no recent competitive matches
  (same condition as HS%) or when no fetched match carried a stats entry for them. It shares
  HS%'s per-match cache and the session-lived cache (renamed `RecentStatsCache`, still keyed by
  puuid + newest competitive match id) and the same "shown for incognito players too" rule,
  since both figures come out of one download. `PlayerRow.kd: Option<f64>` — the float is why
  `PlayerRow`/`TrackerSnapshot` derive `PartialEq` but no longer `Eq`; snapshot dedup only ever
  used `PartialEq`, and the value is a finite ratio.
- **Vandal + Phantom skins** — `GET /core-game/v1/matches/{id}/loadouts`, 1 request/match,
  **INGAME only** (pregame/menus → `null`). Path `Loadouts[].Loadout.Items["<weapon
  uuid>"].Sockets["bcef87d6-…"].Item.ID` → skin uuid → `/v1/weapons/skins` cache for name +
  icon. Vandal `9c82e19d-4575-0200-1a81-3eacf00cf872`, Phantom
  `ee8e8d15-496b-07ac-e5f6-8fae5d4c7b1a` (from `/v1/weapons`). All in `constants.rs`.
- **Stats for incognito players ARE fetched and shown** (puuid is known; vRY does this) — only
  name/level stay hidden. Applied in `assemble.rs` (no incognito gate on the stat fields).
- **Row ordering (user decision).** The backend now owns ordering: ally block (`is_ally`)
  first then enemies; `is_self` first within the ally block; deterministic within each block
  by display name (case-insensitive, hidden/unresolved names last) tie-broken by puuid. The UI
  colours by `is_ally` only and must not re-sort or read the raw Red/Blue `team` id as a
  colour. Implemented in `assemble::order_rows`, contract documented in `ipc-contract.md`.

**Request-count math at match start (10 players, all uncached, INGAME):** unchanged phase-1
core (1 coregame players-id + 1 coregame match + 1 name-service batch + 10 MMR) **plus phase
2**: 10 competitiveupdates + up to 10×5 = 50 match-details + 1 loadouts = **61 new requests**.
All routed through the existing 429 retry with a `INTER_REQUEST_DELAY_MS = 120` pause between
dispatches. match-details are the bulk of that (~45 calls in a measured live comp match) and
its slowest calls, so they go out **two at a time** (`MATCH_DETAILS_CONCURRENCY = 2`), which
counts as one dispatch for the 120 ms pause; every other call stays one per dispatch. Both
lanes consult the session 429 gate before sending and arm it on a limit, so an armed backoff
holds them both. A match-details payload is downloaded **once per match id**, not once per
player (`MatchDetailsCache`, session-lived, keeps only the parsed per-player totals), so lobby
members who played together share the download and a retry pass refetches only the ids that
failed. Re-entry for the same match (score changes every round) costs **one** GET — the
coregame players-id call, kept as the change detector; the roster/map/agents come from
`MatchCache`, and returning players skip match-details via `RecentStatsCache`. WR adds **0** requests (reuses phase-1 MMR), and so does KD (it reads the
match-details already fetched for HS%).

**Agent-select steady state (PREGAME):** **1 request per tick** — `pregame/v1/matches/{id}`,
with the match id served from cache — and **0 once every ally has locked in**, since the tick
stops there (see the pregame tick notes in the previous section). The 5 ally rows themselves are fetched once, on
the build that enters agent select, and reused by every later tick.

### Review-pass fixes (2026-08-24)

Applied after a Fable review of the first backend cut:

- **Lightweightness — per-match name/MMR cache.** `app_state::MatchCache` keys resolved
  names + MMR by match id; an in-match presence update (the score changes every round) reuses
  the cache and does **not** refetch. Invalidated on a new match id or any transition to MENUS.
  The poke channel is drained (`try_recv` loop) before each rebuild so a burst of presence
  events collapses into a single snapshot build.
- **Reconnect handling.** After every websocket drop the listener task injects a synthetic
  poke so the session re-polls presence (transitions during the outage aren't lost); publishes
  a "Reconnecting…" not-running snapshot immediately (no stale INGAME table); checks lockfile
  re-reads and compares the whole lockfile each retry (pid/port/password change = stale
  session) to bail at once when the client is gone **or replaced**; and resets the reconnect
  backoff after a successful connection.
- **Token expiry.** `fetch_names`/`fetch_all_mmr` now return `Result` and propagate
  `BAD_CLAIMS` so the initial paint **and** the event loop route it through the same
  refresh-tokens-and-retry-once arm (previously the initial snapshot and the name/MMR paths
  swallowed it).
- **429 retry.** All pd **and glz** calls (names, MMR, competitive-updates, match-details,
  loadouts, pregame/coregame match-id + match payload) share one 429 wrapper: one backed-off
  retry honoring the server's `Retry-After` header (delay-seconds form, capped at 30s) and
  falling back to ~6s. 401/403 responses map to the `BAD_CLAIMS` refresh signal
  (refresh-once-then-fail). The wrapper sleeps inside the build, so the 1s pregame poll
  cannot stack attempts while rate-limited. The backoff is also recorded as one session-wide
  deadline (`RateLimitGate`, pd + glz together, since Riot limits the client): every later
  call waits out whatever is still owed, so abandoning a request mid-backoff — which a
  transition does — cannot resend inside the server's window. (Audit hardening, 2026-08-25.)
- **404-race timing.** coregame retries once after ~5s, pregame retries immediately (was 2s
  for both), per §10.5.
- **Malformed presence.** A single un-parseable presence entry in a websocket batch is skipped,
  not treated as aborting the whole event.
- **Empty own presence.** An absent own presence, or one with an empty `private` blob, surfaces
  `NotReady` (poll again) instead of rendering as MENUS.
- **Deps trimmed** (lightweightness): dropped the unused `tracing` dep and the duplicate
  `serde_json` dev-dependency; `tokio` narrowed from `full` to
  `rt-multi-thread,macros,time,sync,net`; `reqwest` to `default-features = false` +
  `json,native-tls`. `tauri-plugin-opener` kept (planned tracker.gg link opening).

### Phase-2 review fixes (2026-08-24)

Applied after a Fable review of the phase-2 cut (`app_state.rs` + a few pure helpers):

- **Cache freshness now keyed on state, not just match id (HIGH).** Pregame and coregame
  share the same match GUID, so a cache built in PREGAME (5 allies, no loadouts) was wrongly
  reused verbatim INGAME — enemies rendered empty and Vandal/Phantom skins stayed null all
  match. `MatchCache` now stores `ingame` + `enriched`; `is_fresh_for(match_id, ingame)`
  treats a pregame cache as **stale** once INGAME. On the pregame→ingame upgrade the already
  fetched ally rows (names/MMR/updates/HS) are **reused** (`begin_match` keeps same-match
  data); only the newly visible enemies + the loadouts are fetched. Unit-tested.
- **Two-phase emit (HIGH).** The first in-match snapshot no longer waits on the whole
  sequential burst (10 competitiveupdates + up to 30×500 KB match-details + loadouts). Phase 1
  publishes as soon as names + MMR are in (ranks/RR/peak/WR — WR is free); phase 2 fetches
  updates/HS/skins and publishes the enriched snapshot. The dedup'd `tracker-state` event
  carries both. Documented for the UI in `ipc-contract.md`.
- **Enrichment is interruptible (HIGH).** The phase-2 burst runs inline in the session loop;
  it now drains the poke channel between per-player requests and aborts (rebuilding for the
  current state) when a new presence event arrives, so a dodge/transition during phase 2
  surfaces promptly instead of blocking behind the remaining match-details. Partial phase-2
  results stay cached, so the rebuild only finishes the missing work.
- **BadClaims mid-burst (MEDIUM).** Chose the simpler mitigation: **proactively refresh the
  token before an uncached/upgrade burst** (best-effort local call). A stale token is the
  common between-matches case, so this removes the mid-burst BadClaims redo without the
  plumbing of partial-store-then-propagate. (Phase-1 results are also stored before phase 2,
  so even a late BadClaims doesn't redo names/MMR.)
- **Round half-to-even (MEDIUM).** `compute_win_rate` and `HitCounts::headshot_percent` use
  `round_ties_even()` to match Python `round()` exactly (12.5 → 12, not 13). Tie-case tests
  added (1/8 games, 1/8 headshots).
- **Sleep placement (LOW).** The 120 ms inter-request delay now sits only *between* requests
  — no trailing sleep after the last request of a loop, none after a failure or a
  cache-hit/skip. MMR fetches got the delay too (were missing it), matching the spec's
  per-player 120 ms.
- **Skin cache trimmed (LOW).** Static data now fetches `/v1/weapons` and keeps only the
  Vandal + Phantom `skins` arrays (matched by parent weapon uuid) instead of storing all ~5k
  skins from `/v1/weapons/skins`. Disk cache stays version-keyed.
- **Row-order key (LOW).** `assemble::order_rows` uses `sort_by_cached_key` so each row's
  lowercased-name key is computed once, not per pairwise comparison.

### Contract-alignment pass (2026-08-24)

Three backend/doc mismatches flagged by the UI builder against `ipc-contract.md`, fixed on the
backend side (`ipc-contract.md` updated to match):

- **`enriched` flag on `TrackerSnapshot` (new field).** Added `enriched: bool` (serde
  `enriched`) so the UI keys loading skeletons off an explicit flag instead of inferring "still
  loading" from data absence. It is `false` **only** on the fast phase-1 snapshot of a match
  whose heavy stats are still being fetched; `true` on the enriched phase-2 snapshot, on a
  re-entry snapshot of an already-loaded match (the fully-cached path), and on every non-match
  state (Menus / ValorantNotRunning, where there are no players). Wired through
  `app_state::assemble_snapshot` (new `enriched` param: `false` for phase 1, `true` for the
  enriched and fully-cached publishes) and set `true` in the Menus and `not_running`
  constructors. The two snapshots already differed on their heavy fields, so this rides the
  existing dedup unchanged.
- **Incognito `accountLevel`.** `assemble.rs` previously withheld the level only on the
  `hide_account_level` flag; the contract also withholds it for incognito players. `level_visible`
  is now `(!incognito && !hide_account_level) || is_self || is_party_of_self` — the self row is
  never nulled regardless of flags, and party members still always see the real level. Unit test
  `incognito_hides_account_level_except_for_self` added (self+incognito keeps its level, non-self
  incognito → null). Wire caveat documented in `ipc-contract.md`: Riot itself zeroes coregame
  `PlayerIdentity.AccountLevel` when the hide flag is set (even for self), so a self level may
  still read `0` in-match off the wire; the backend passes it straight through and does **not**
  backfill from MENUS presence.
- **`agentSelectionState` is a raw string, not an enum.** No code change — the Rust type was
  already `Option<String>` passing Riot's `CharacterSelectionState` straight through.
  `ipc-contract.md` corrected to document it as `string | null` with `"locked"`/`"selected"` as
  the known values and other strings possible.

### Incremental stat loading (2026-08-25)

The two-phase emit became a settle-point emit: the table is published as each stat lands
instead of at the two phase boundaries, and each row carries a `PendingStats` group map
(`name` / `rank` / `history` / `recentStats` / `skins`) so the UI skeletons individual cells.

- **Emission points** (`app_state::build_match_snapshot` and the two fetch phases). Forced:
  the name batch resolving (the first paint), the end of phase 1, and the final publish.
  Coalesced: each MMR insert, each competitiveupdates insert, each `recent_stats` insert (per
  *player*, not per match-details download), and the loadouts landing.
- **Coalescer.** `PROGRESS_COALESCE_MS = 250`: a `ProgressGate` holding the last emit's
  `Instant` lets an emit through when it is forced or the window has passed. No dirty tracking
  is needed because every phase ends in a forced flush, so a swallowed emit is never the last
  word. The decision is the pure `should_emit(elapsed, forced)`, unit-tested. Worst case is a
  handful of events per second during the opening burst.
- **Pending rule** (`assemble.rs`). A group is pending iff its cache map lacks the row's entry
  **and** the build is not the final one (`AssembleInput::finality`); `skins` is per-match
  (`ingame && !loadouts_fetched`) because loadouts arrive for the whole roster at once. Row
  ordering and the privacy withholding are untouched.
- **`enriched` reworked** to "every stat has settled", with the invariant `enriched == true`
  ⇒ no row carries a pending flag. `assemble_snapshot` passes the one flag as both the
  snapshot's `enriched` and the assembler's `finality`, so the invariant holds by
  construction; unit-tested against an empty cache.
- **`SnapshotParts`** carries the fixed match context (roster, parties, map, mode, status,
  static data, season) so a phase can hold `MatchCache` mutably and still publish — the
  borrow conflict that made mid-loop emits awkward.
- **Request counts and retry/finality semantics are unchanged.** Every fetch, retry, cache
  lookup, inter-request delay, rate-limit gate, poke cancellation and `enrichment_is_final`
  rule is byte-for-byte what it was; only emission timing and the new pending metadata
  changed. The existing tests guard this.

### Two-lane match-details (2026-08-26)

A live competitive match measured ~45 match-details calls taking ~23 s of wall clock, all
sequential. A full log of that burst recorded **zero** 429 responses, so the deliberate
one-at-a-time caution was relaxed to two.

- **`MATCH_DETAILS_CONCURRENCY = 2`** (`app_state`). A player's HS%/KD window is resolved
  against the caches first (`plan_window`, pure), then the ids left over are downloaded in
  batches of two. Nothing else about the burst changed: competitiveupdates, MMR and loadouts
  stay one request at a time.
- **Pacing kept, per dispatch.** The `INTER_REQUEST_DELAY_MS = 120` pause now sits between
  *dispatches*, a two-wide batch counting as one — the simpler of the two options, and it keeps
  the spacing the gate design assumes rather than leaning on the 429 retry alone.
- **429 semantics unchanged.** Both lanes run through `with_rate_limit_retry`, so each waits
  out an armed deadline before sending and arms `RateLimitGate` on a limit of its own; a
  backoff armed by one lane holds the other's retry and every later batch. A poke still
  abandons the whole batch (`until_poke`), and whatever landed before it stays cached.
- **In-flight dedup.** `plan_window` lists each id once, so two lanes can never be sent after
  the same match — which would spend the request twice and fold its totals in twice.
- **Downstream untouched.** `MatchDetailsCache`, `RecentStatsCache` and the per-player settle /
  `Partial` reporting are byte-for-byte what they were; only the dispatch shape changed.

### TLS

- Local HTTPS client (`reqwest`) and the local websocket (`tokio-tungstenite` + `native-tls`)
  both accept the self-signed cert. These clients are only ever pointed at `127.0.0.1`. The
  static-data + any public fetches use a separate ordinary client with normal cert validation.

### Testing

- 169 unit tests today (the last four cover the all-locked roster predicate, the agent-select
  match-id cache and the deduped 404 retry); the list below was written at 92 (86 as below + 6 from the phase-2
  review fixes: the `MatchCache` freshness rule incl. the pregame-stale-when-ingame guard and
  same-match reuse, plus round-half-to-even tie cases for WR and HS%), all pure functions, driven by inline JSON fixtures
  authored from this spec's documented shapes (phase-2 fixtures sanitized from the live probe
  captures — fake puuids/names/skin ids, real numeric values): lockfile parsing, presence
  decode (nested + flat + custom-game + LoL skip
  + party grouping + `partyClientVersion` + empty-`private` not-ready), tier→name mapping,
  current/peak rank (incl. before-Ascendant +3 shift), coregame/pregame extraction,
  name-service parse, content season selection, host/shard construction (incl. `shared` shard),
  remote error mapping, static-data lookups (incl. skin resolution + icon fallback), websocket
  event parsing, the per-match cache invalidation, the session-lived HS% cache keying, and the
  full `assemble_players` privacy + rank + ordering rules. Phase-2 additions: WR derivation
  (incl. 0-games null + rounding), ΔRR + last-5 pips (incl. 0-RR `Unknown` ambiguity), HS%
  math (hand-computed 9/26/1 → 25% from the real capture, then sanitized) incl. cross-match
  accumulation, loadout skin extraction (Vandal + Phantom), the HS% single-fetch cache, and
  row ordering (self-first, ally-block-first, deterministic name/puuid tiebreak). The KD pass
  adds 5 more (cross-match accumulation ignoring other subjects, 2-decimal rounding, the
  zero-deaths case, no-stats → `None`, and HS%+KD from one payload) plus a null-both assemble
  case: **119 tests** in total.
- **Not covered by tests (requires a live client):** the async orchestration loop in
  `app_state.rs`, the actual HTTP/WS calls, token-refresh-on-`BAD_CLAIMS` round trips, and the
  real end-to-end state transitions. These compile and are structured for graceful degradation
  but have not been exercised against a running game.

### Open items needing live-game verification

1. `latam`/`br`/`pbe` shard mapping produces correct pd/glz hosts.
2. The last `competitivetiers` array entry is genuinely the current act's tier table (icons
   match the in-game client).
3. Both presence shapes (nested/flat) are handled correctly against whatever Riot serves now.
4. 404/`RESOURCE_NOT_FOUND` retry timing and `BAD_CLAIMS` refresh behave in real transitions.
5. Party grouping across all online presences matches the in-game party colours.
6. (Phase 2) The match-start stat burst (61 requests for a 10-player lobby) stays under Riot's
   rate limit with the 120 ms inter-request delay + 429 retry — the probe captures were solo/
   2v2, so the full-lobby throughput is untested. Loadouts were verified for 4 players; the
   Phantom weapon id + a Phantom-equipped player were only cross-checked via valorant-api, not
   a live 10-player loadouts payload.
7. **Pregame vs coregame match-id equality.** The phase-2 cache now assumes the PREGAME and
   INGAME endpoints report the **same** match GUID for one match (so a pregame-built cache is
   explicitly treated as stale — not merely absent — once INGAME; see the "Phase-2 review
   fixes" note). This equality is the standard community understanding but was not confirmed
   against a live pregame→ingame transition on this build. Verify that
   `pregame/v1/players/{puuid}.MatchID` equals the subsequent
   `core-game/v1/players/{puuid}.MatchID`. If they ever differ, the pregame→ingame data reuse
   still works by match-id fallback but the freshness guard becomes a no-op (harmless — the
   ingame build just refetches everything).

### UI revision 2 — contract additions (2026-08-24)

Two backend changes requested by `ui-spec.md` "Revision 2":

- **`accountLevel` 0 means hidden.** `assemble.rs` now emits `null` for a wire `AccountLevel`
  of 0 (Riot zeroes it when the hide-level flag is set — even on your own row, confirmed in
  live verification round 2) instead of passing the 0 through. `accountLevel` is therefore
  never 0 on the wire to the UI: a real level or `null`.
- **`peakRankAct`** (`PlayerRow.peak_rank_act`): short label for the act the peak rank was set
  in, e.g. `"E6: A3"` / `"V26: A1"`. `content::act_label` ports vRY's
  `Content.get_act_episode_from_act_id` — walk the content-service season list, read the
  matching act's trailing name token (Roman or Arabic), and pair it with the last `episode`
  entry seen before the act's own episode; the episode keeps its identifier verbatim when it
  already mixes letters and digits (the V-era `V26` naming), otherwise it gets an `E` prefix.
  One deliberate improvement over vRY: the newest act has no episode entry after it, where vRY
  leaves the episode unset and prints a literal `None`; we fall back to the last episode seen.
  The label is `null` when `rank::compute_peak` attributes no season to the peak (peak == the
  player's current tier). vRY instead falls back to the *current* act there — worth revisiting
  if the blank cell reads wrong in a live lobby. The season list is now kept on `Session`
  (`seasons`) and passed through `AssembleInput::seasons`; the season id is derived from it
  rather than from a second parse.

## Live verification (2026-08-24, NA account, in-menus)

Probed every endpoint against the running client and cross-checked against a vRY console row for the same account. Raw captures in session scratchpad (not committed).

Confirmed:
- Lockfile, local auth, `/entitlements/v1/token`, `/riotclient/region-locale` (region `NA`), `/chat/v4/presences`, `/chat/v1/session`: all work as specced.
- Private presence is currently the NESTED shape (`matchPresenceData` / `partyPresenceData` / `playerPresenceData` / `premierPresenceData`). Values matched vRY exactly: accountLevel, competitiveTier 21, partyId, sessionLoopState MENUS.
- `/mmr/v1/players/{puuid}`: current-season tier 21 + RR 36 = vRY "Ascendant 1 (36)". Peak from `WinsByTier` max across seasons = 22 (Ascendant 2) = vRY. vRY's WR figure = current-season `NumberOfWins/NumberOfGames` (8/14 = 57%), NOT all-time match history.
- `/mmr/.../competitiveupdates?queue=competitive`: `RankedRatingEarned` of newest match = vRY's ΔRR (+13). Last-5 W/L derivable from `RankedRatingEarned` sign (note: 0-RR edge cases exist).
- `name-service/v2/players` (PUT, puuid array): returns GameName/TagLine.
- `match-history/v1/history` + `match-details/v1/matches/{id}` (~500 KB per match): respond fine; HS% source confirmed available.
- valorant-api.com `/v1/competitivetiers`: LAST array entry is the current tier table (tier 21 = ASCENDANT 1, icons present). Use last entry. (Resolves implementation-notes gap.)

New finding — client version staleness: valorant-api.com reported `...shipping-18-5304478` while the actual running client is `...shipping-20-5340415` (visible in presence `partyPresenceData.partyClientVersion`). pd accepted the stale header today, but on patch days Riot may reject it. Recommendation: prefer the version from own presence (or local session data) once connected, with valorant-api.com as bootstrap/fallback.

Still needing live verification later: latam/br->na shard mapping (NA account can't test), flat presence shape in the wild, 404-race + BAD_CLAIMS timing during real pregame/ingame transitions, party color grouping with an actual party, ingame/pregame endpoints themselves (was in menus during probe).

## Live verification round 2 (2026-08-24, in-game, NA, 2v2 skirmish)

Probed from inside a running match; all values cross-checked against vRY console output for the same match.

Confirmed:
- glz host construction `glz-na-1.na.a.pvp.net` works as built.
- `GET /core-game/v1/players/{puuid}` -> MatchID; `GET /core-game/v1/matches/{id}` -> `Players[]` with `Subject`, `TeamID` (Red/Blue), `CharacterID`, `PlayerIdentity.{Incognito,HideAccountLevel,AccountLevel}` exactly as specced. Incognito and hide-level flags matched vRY's rendering (hidden name shown as agent, levels blanked; own level reads 0 in-match when hidden).
- `GET /core-game/v1/matches/{id}/loadouts` (~79 KB for 4 players): shape is `Loadouts[] -> {Subject, CharacterID, Loadout.Items}`. Vandal = item key `9c82e19d-4575-0200-1a81-3eacf00cf872`; skin uuid at socket `bcef87d6-209b-46c6-8b19-fbe40bd95abc` -> `.Item.ID`; resolves via valorant-api `/v1/weapons/skins/{uuid}` (displayName). Resolved all 4 players' Vandal skins matching vRY exactly (Neptune/Mystbloom/Reaver/Aeris). Tier-2 skin column fully validated.
- Cross-player MMR fetch (`/mmr/v1/players/{other-puuid}`) works with own tokens.

Still open: latam/br shard mapping, flat presence shape, 404-race/BAD_CLAIMS timing at real transitions, party grouping with an actual multi-player party (this match was solo, partySize 1).
