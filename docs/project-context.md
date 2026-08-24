# Project Context

Last updated: 2026-08-24

## Goal

A lightweight Windows desktop app with a good UI that shows an in-match player table for the user's current Valorant match — like VALORANT-rank-yoinker (vRY), but a real GUI instead of a console.

## Decisions made

| Decision | Choice | Why |
|---|---|---|
| Backend approach | Build our own against Riot's local client API directly (option 3) — no vRY code dependency | vRY is a console app, not a library; the plumbing we need is small (a few hundred lines); endpoints change rarely |
| Correctness reference | vRY source code | User verified vRY's data is accurate in real matches |
| ValoTracker | Do not reuse its backend logic | User verified its data is incorrect |
| Static data (icons, rank/skin/agent names, images) | valorant-api.com | Free, no key, same source vRY uses |
| Scope v1 | In-match player table only | User request |
| Discord RPC | Not now, keep architecture open for it later | User request |
| Tech stack | **DECIDED: Tauri 2 + React + TypeScript + Vite + Tailwind CSS v4** | Small native footprint, good UI story, single-language plumbing behind the Rust backend |

## Architecture intent (regardless of stack)

- Thin native/backend layer: read Riot lockfile, call local client HTTPS endpoints (self-signed cert, basic auth) + remote pd/glz endpoints, listen on local websocket for match state.
- All interesting logic (presence parsing, tier→rank mapping, table assembly) lives in one language behind an adapter interface so the plumbing is swappable.
- UI reads from that layer; keep a seam where Discord RPC could plug in later.

## Status / next steps

1. ~~User picks stack (Tauri vs Python).~~ Done — Tauri 2.
2. ~~Scaffold project.~~ Done — see Repo layout below.
3. ~~Implement lockfile + local API auth, then presence → player list → ranks pipeline.~~
   **Done — Rust backend implemented in `src-tauri/src/riot/` + `app_state.rs`.** All parsing
   is pure and unit-tested. Exposes two Tauri commands (`start_tracker`,
   `get_tracker_state`) and one event (`tracker-state`). The exact TS-facing contract is in
   `docs/ipc-contract.md`.
3a. ~~**Phase 2 — per-player stats.**~~ **Done.** Added the five `ui-spec.md` tier-2 columns:
   WR (free from phase-1 MMR), ΔRR + last-5 W/L (competitiveupdates), HS% (match-details,
   reusing competitiveupdates match ids; session-cached per puuid+newest-match), and
   Vandal/Phantom skins (coregame loadouts + valorant-api skin cache). New modules
   `riot/stats.rs`, `riot/loadout.rs`. Backend now also **owns row ordering** (ally block
   first, self first, deterministic name/puuid tiebreak; UI colours by `isAlly` only). New
   `PlayerRow` fields documented in `docs/ipc-contract.md`; phase-2 notes + request-count math
   in `docs/backend-spec.md`. 86 unit tests. `cargo check` / `cargo test` /
   `cargo clippy --all-targets -- -D warnings` all pass; `pnpm build` still passes (frontend
   untouched). Live-verification open items for the full-lobby stat burst are listed in the
   backend spec.
4. ~~**UI.**~~ **Done — built from `docs/ui-agent-prompt.md` in a user-run session.** `src/`
   now holds the whole frontend: IPC wiring (`src/ipc/`, `src/hooks/useTracker.ts`), the
   two-team player table with all eleven columns, and the non-match state screens. Only
   `src/` and `index.html` were touched. `pnpm build` (tsc + vite) passes; `pnpm tauri dev`
   opens the window and shows "Waiting for VALORANT" with the game closed. Layout decisions
   and the points flagged against the IPC contract: `docs/ui-spec.md` -> "Implemented (v1)".
   **Next: the Fable code review of the frontend.**
5. **Open — needs live game to verify** (backend could not integration-test; Valorant not
   running on this machine): `latam`/`br` → `na` shard mapping; that the *last*
   `competitivetiers` table entry is the current one; and that the presence nested-vs-flat
   dual paths both fire in the wild. See the new "Implementation notes" in `docs/backend-spec.md`.
   The UI has the same gap: it was verified against fixture data at 1000x700 (fits with no
   scrolling, no horizontal overflow) and against the real `ValorantNotRunning` snapshot, but
   a live ten-player lobby — party colours, the fast → enriched hand-off, real skin art — has
   not been seen yet.

## Repo layout

Scaffolded with `pnpm create tauri-app` (react-ts template), files at the repo root.

```
.
├── index.html            # Vite entry HTML (window title set here)
├── package.json          # Frontend deps + scripts (pnpm)
├── pnpm-workspace.yaml    # pnpm settings (allows esbuild build script)
├── vite.config.ts         # Vite config; React + @tailwindcss/vite plugins
├── tsconfig.json
├── public/                # Static assets served as-is
├── src/                   # React + TypeScript frontend
│   ├── main.tsx           # React entry, imports index.css
│   ├── App.tsx            # Shell: useTracker() -> header + the screen for the status
│   ├── index.css          # Tailwind v4 entry + theme tokens + the waiting-pulse keyframes
│   ├── ipc/               # types.ts (mirror of the Rust shapes), tracker.ts (2 commands + 1 event)
│   ├── hooks/useTracker.ts# start -> initial snapshot -> event subscription, newest wins
│   ├── lib/               # table.ts (columns + team tints), players.ts, format.ts, profile.ts
│   └── components/        # Header, StatusScreen, PlayerTable, TeamBlock, PlayerRow, cells/
└── src-tauri/             # Rust / Tauri 2 backend (Riot pipeline implemented)
    ├── Cargo.toml         # + reqwest, tokio, tokio-tungstenite, native-tls, base64, thiserror
    ├── tauri.conf.json    # productName, identifier com.connor.valorant-tracker, 1000x700 window
    └── src/
        ├── main.rs        # Calls valorant_lightweight_tracker_lib::run()
        ├── lib.rs         # Tauri builder: manages TrackerState, commands + `tracker-state` event
        ├── app_state.rs   # Orchestration state machine (connect/reconnect, emit snapshots)
        └── riot/          # All Riot-API logic (pure parsers + IO clients)
            ├── mod.rs
            ├── constants.rs   # NUMBER_TO_RANK, before_ascendant_seasons, gamemodes, headers, shard map
            ├── types.rs       # TrackerSnapshot / PlayerRow / RankInfo … (the frontend-facing shapes)
            ├── error.rs       # Error taxonomy (game-not-running is NOT an error)
            ├── lockfile.rs    # find + parse lockfile, basic-auth header
            ├── local_api.rs   # local HTTPS (self-signed): entitlements, presences, region-locale
            ├── remote_api.rs  # pd/glz/shared clients: headers, host construction, error mapping
            ├── websocket.rs   # local wss listener + presence-event parsing
            ├── presence.rs    # decode private presence (nested + flat), party grouping
            ├── match_state.rs # pregame/coregame player extraction
            ├── names.rs       # batch name-service resolution
            ├── rank.rs        # MMR parse, current/peak rank, before-ascendant tier shift, win rate
            ├── stats.rs       # phase 2: competitiveupdates (ΔRR + last-5) + match-details HS% math
            ├── loadout.rs     # phase 2: coregame loadouts -> Vandal/Phantom skin ids
            ├── content.rs     # content-service season list (current/previous act)
            ├── static_data.rs # valorant-api fetch + version-keyed disk cache + lookups (+ skins)
            └── assemble.rs    # display-ready PlayerRows (privacy, stats, guaranteed row order)
```

Frontend styling uses Tailwind CSS v4 via the Vite plugin (`@tailwindcss/vite`) — no
`tailwind.config.js` or PostCSS needed; utilities come from `@import "tailwindcss";` in
`src/index.css`.

### How to run

- `pnpm install` — install frontend deps (first run also compiles the esbuild binary).
- `pnpm tauri dev` — run the desktop app in dev (spins up Vite + the Rust shell with hot reload).
- `pnpm tauri build` — produce a release build + installer.
- `pnpm build` — frontend-only typecheck (`tsc`) + Vite production build (output in `dist/`).
- `cd src-tauri && cargo check` — typecheck the Rust side.

Note: `cargo`/`rustc` are not on PATH by default in this environment; prepend the rustup
cargo bin dir first (PowerShell: `$env:Path = "$env:USERPROFILE\scoop\apps\rustup\current\.cargo\bin;$env:Path"`).

## Conventions

- Python work: always in a venv. Node work: pnpm.
- Keep docs/ updated whenever decisions or scope change — this folder is the cross-session memory.
