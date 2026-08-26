# Testing / debug mode

Three dev-only affordances for working on the app. All are absent from release builds: the
frontend branch sits behind `import.meta.env.DEV` (tree-shaken out of `dist/`), and the backend
capture plus the console log behind `#[cfg(debug_assertions)]` (never compiled into a release
binary).

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

## Git

`/public/debug-snapshot.json`, `/debug/` and `/src-tauri/debug/` are gitignored, so both the
default capture directory and the sample snapshot stay out of the repo. Point
`VLT_DEBUG_CAPTURE` somewhere else and it is on you to keep it untracked.
