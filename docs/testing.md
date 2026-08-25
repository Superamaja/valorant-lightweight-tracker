# Testing / debug mode

Two dev-only affordances for working on the app without a live match. Both are absent from
release builds: the frontend branch sits behind `import.meta.env.DEV` (tree-shaken out of
`dist/`), and the backend capture behind `#[cfg(debug_assertions)]` (never compiled into a
release binary).

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

## Git

`/public/debug-snapshot.json`, `/debug/` and `/src-tauri/debug/` are gitignored, so both the
default capture directory and the sample snapshot stay out of the repo. Point
`VLT_DEBUG_CAPTURE` somewhere else and it is on you to keep it untracked.
