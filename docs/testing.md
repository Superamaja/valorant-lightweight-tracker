# Testing / debug mode

Three dev-only affordances for working on the app. All are absent from release builds: the
frontend branch sits behind `import.meta.env.DEV` (tree-shaken out of `dist/`), and the backend
capture plus the console log behind `#[cfg(debug_assertions)]` (never compiled into a release
binary). The diagnostics report at the end is the exception: it ships in every build, and it is
what a user can send when the console log is not there to read.

## 1. UI-only testing in a plain browser

Put a `TrackerSnapshot` JSON (hand-written against `docs/ipc-contract.md`, or captured below)
at `public/debug-snapshot.json`, then:

```
pnpm dev
```

Open the localhost URL Vite prints (default <http://localhost:1420/>). `useTracker` fetches
`/debug-snapshot.json` first and, on a valid JSON response, renders it and **skips all Tauri
IPC** — so the UI runs in an ordinary browser tab, where the Tauri APIs do not exist. Edit the
JSON and reload to iterate on states (Pregame, incognito rows, `enriched: false` skeletons,
missing stats).

With no such file, Vite answers with the `index.html` SPA fallback rather than a 404, so the
loader checks the response's content type (and guards the `.json()` parse). Anything that is
not JSON means "no debug snapshot" and the hook falls through to the normal IPC path silently
— `pnpm tauri dev` behaves exactly as before.

The converse also holds: the Tauri dev window runs off the same Vite dev server, so **while
`public/debug-snapshot.json` exists, `pnpm tauri dev` renders the debug data too**, not live
data, with no visual tell. Delete or rename the file to go back to live — the convention is prefixing
an underscore (`_debug-snapshot.json`, also gitignored) so it can be flipped back later. (Release builds are
unaffected either way — the loader is compiled out.)

### Preview fixture

`docs/fixtures/preview-snapshot.json` is a hand-made Ingame lobby with fake names and ids (asset
URLs only) covering the states that are hard to catch live: a draw pip, an unknown pip,
empty pip slots, a partial HS%/KD window (4 of 5), a short one (3 of 3, and 1 of 1), full
rows and N/A rows. It lives outside `public/` so it never ships in the exe. It loads through the same file as any other snapshot. Park the current
one under the underscore name and copy the fixture in:

```powershell
Move-Item public\debug-snapshot.json public\_debug-snapshot.json
Copy-Item docs\fixtures\preview-snapshot.json public\debug-snapshot.json
pnpm dev
```

### Real capture fixture

`docs/fixtures/ingame-snapshot.json` is a real 10-player enriched Ingame capture, committed
so a fresh clone on another machine has live-shaped data to test against. It holds real
Riot IDs from that lobby (kept deliberately). Load it the same way as the preview fixture,
copying it to `public/debug-snapshot.json`.

### Multi-team fixture

`docs/fixtures/multiteam-snapshot.json` is a hand-made Gauntlet: Glitched lobby: 16 players
(`Player 1`..`Player 16`, fake ids) in 8 duos with `group` set, the local player in group 3
(so the own group leads and the rest follow ascending). It previews the flat "Players" list
and the per-team portrait tints. Load it the same way.

then open <http://localhost:1420/>. To go back, move `_debug-snapshot.json` back over
`debug-snapshot.json` (or delete it for live data).

## 2. Capturing real snapshots from a live game

```powershell
$env:VLT_DEBUG_CAPTURE = "debug"
pnpm tauri dev
```

Every published snapshot is also written as pretty JSON to that directory (relative paths are
relative to `src-tauri/`, so `"debug"` lands in `src-tauri/debug/`; pass an absolute path to
put it elsewhere). Files are named `snapshot-{counter:04}-{status}.json`, e.g.
`snapshot-0007-Ingame.json`, counter in emission order. Each rebuild also dumps the raw
`/chat/v4/presences` response body as `presences-{counter:04}.json` (undecoded, so the full
roster membership and unknown fields survive — used to diagnose party grouping). The counter
is shared between both kinds, so files interleave in write order. Writing is best-effort: any
failure is ignored and never affects the app.

Play a match, quit, then pick a file out of that directory and copy it over for UI work:

```powershell
Copy-Item src-tauri\debug\snapshot-0007-Ingame.json public\debug-snapshot.json
```

Captured snapshots contain real puuids and player names. Do not share them.

## 3. Live console log

Always on in a debug build, with nothing to configure. Every line goes to stderr, so it shows
up in the terminal running `pnpm tauri dev` (a debug exe launched from a terminal prints there
too; launched from Explorer there is no console to print to).

```
[vlt     4.812] state: Pregame -> Ingame  map=Ascent mode=Competitive players=10 enriched=false
[vlt     4.930] net: #37 GET /mmr/v1/players/8f4c1d2e -> 200 (117ms)
```

The number is seconds since the first line. Categories:

- `state` — every snapshot that survived the dedup and reached the UI, plus how many rows are
  still waiting on data.
- `rebuild` — why a rebuild started: a presence poke, the agent-select tick, a retry backoff,
  or an interruption mid-enrichment.
- `net` — one line per remote request (serial number, path, HTTP status, round trip), plus 429
  backoffs armed and waited out.
- `enrich` — what each phase set out to fetch, cache hits, the loadout/chroma summary, and what
  a partial pass left missing.
- `presence` — why a rebuild found the local presence unusable: own presence missing from the
  roster, its `private` blob still empty, or the local endpoint answering "not ready".
- `ws` / `conn` — websocket connect/close/reconnect, and the session lifecycle: lockfile found,
  session up, token refreshes, connection lost.

Lines carry truncated puuids and match ids, never full ones: an id keeps its first 8
characters, and in a request path the query string is dropped. Map/mode and the client
version are real. `cargo build --release` compiles the whole thing out, arguments included.

### Standalone debug exe

```powershell
pnpm tauri build --debug --no-bundle
```

builds `src-tauri/target/debug/valorant-lightweight-tracker.exe`: a debug-profile build with
the release-shaped frontend baked in (real CSP, no Vite dev server) but all three affordances
above still compiled in, plus devtools (Ctrl+Shift+I). Run it from a terminal to get the live
console log without `pnpm tauri dev`; it is also the artifact for the release-shaped CSP smoke
test (see roadmap).

## Raw API snapshot (no build needed)

`scripts/snapshot.mjs` dumps the raw Riot API payloads for whatever state the game is in right
now (menus, agent select or in game). It is plain Node with no dependencies and no Rust build, so
it is the first step when a payload needs inspecting, e.g. when adding a new game mode:

```
pnpm snapshot
pnpm snapshot --match-details
```

It reads the lockfile and builds its requests the way `src-tauri/src/riot/` does (same headers,
region to shard mapping, client version from own presence with valorant-api.com as fallback),
makes one request per endpoint, and writes each response as pretty JSON to
`captures/<UTC timestamp>_<state>/`:

- `region-locale.json`, `presences.json` (raw) and `own-presence-decoded.json` (own `private`
  blob, base64 decoded);
- `pregame-player.json` + `pregame-match.json` in agent select, or `coregame-player.json` +
  `coregame-match.json` + `coregame-loadouts.json` in game (with no readable presence state, both
  lookups are tried and the one that answers decides the state);
- `party-player.json` + `party.json`, `mmr.json`, `competitive-updates.json`;
- `pods.json`: glz `/parties/v1/parties/customgameconfigs`, every game pod the region offers
  (its `GamePodPingServiceInfo` keys are what confirms the server-label city tokens);
- with `--match-details`: `match-details.json` for the current match (only exists once it has
  ended), or outside a match for the last played one (`match-history.json`, any queue);
- `manifest.json`: every request with its path, status and error, plus every endpoint skipped
  for the current state. A failed request never stops the run.

The console prints one line per file (ids cut to 8 characters) and then the detected
`sessionLoopState`, `queueId`, `provisioningFlow`, `partyState`, mode id and map id, which is what
identifies a new mode, plus the match's `GamePodID` and the region's pod ids from `pods.json`. It exits non-zero when the lockfile is missing (game not running).

The lockfile password and the access/entitlements tokens are never written or printed, but the
captures hold real puuids, player names and match payloads. `captures/` is gitignored; do not
share them.

## Diagnostics report (all builds)

The release counterpart of the console log above, and the only one users have. `Copy diagnostics`
copies a short plain-text report of what the tracker last saw at each stage (lockfile, local API,
own presence, remote, websocket), plus the app version, OS, WebView2 version, current status and
which screen the user was on. One slot per category, overwritten as it goes: it is a snapshot of
the last thing that happened, not a log.

Where the button is:

- on every status screen, beside the version line (`Waiting for a match`, `Waiting for VALORANT`,
  `Starting`, `The tracker stopped`);
- in the header while a table is up, live or held, left of the update chip.

Both copy the same report; only the `screen` line differs. If the clipboard refuses (rare, but the
users who need this are the ones with broken setups) the status screen prints the report into a
read-only textarea, pre-selected, so it can be copied by hand. The header has no textarea; the
status screen behind it does.

Privacy matches the console log's rule. puuids and match ids keep their first 8 characters, the
lockfile path is printed as the literal `%LOCALAPPDATA%\...` rather than the real one, and the
lockfile password, tokens, entitlements and the presence `private` blob never reach it. Error text
is normalised before it is printed, so URLs (which can carry a puuid) are cut out.

In the plain-browser dev run of section 1 there is no backend, so the button copies a short
`diagnostics unavailable` stub with the app version and the invoke error instead of a report. That
is the expected behaviour there, not a bug.

## Git

`/public/debug-snapshot.json`, `/debug/` and `/src-tauri/debug/` are gitignored, so both the
default capture directory and the sample snapshot stay out of the repo. Point
`VLT_DEBUG_CAPTURE` somewhere else and it is on you to keep it untracked.
