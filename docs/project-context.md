# Project Context

Last updated: 2026-08-26

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
4. ~~**UI.**~~ **Done, reviewed, and revised.** Built from `docs/ui-agent-prompt.md` in a
   user-run session, then Fable-reviewed and iterated: contract-alignment pass (`enriched`
   flag replaces loading inference; backend owns level withholding), Revision 2 (level badge
   on a larger agent portrait instead of a Level column, full-size peak rank icon with
   episode/act label `E6: A3` / `V26: A1`, 48px rows, HS% scope labeled), and live
   agent-select updates (teammates' picks/locks rebuild the pregame roster in real time).
   Pregame team-split bug (players inheriting no TeamID -> everyone rendered as enemy) fixed
   and validated against a real agent-select capture.
5. ~~**Release pipeline.**~~ **Done** (see roadmap "Done" + `docs/release.md`): tag push
   `v*` -> GitHub Actions builds the portable exe onto a Release; `pnpm bump` syncs the three
   version files. **First release tag not yet pushed.**
6. ~~**Debug/testing mode.**~~ **Done** (`docs/testing.md`): UI runs from a JSON snapshot in
   a plain browser (`public/debug-snapshot.json` + `pnpm dev`); debug builds capture real
   snapshots per state change via `VLT_DEBUG_CAPTURE`. Real 10-player enriched captures were
   taken 2026-08-24 and live-verified most of the pipeline (full lobby, skins, act labels,
   hidden levels, unrated-mode rendering, fast->enriched hand-off).
7. ~~**Feature batch 2026-08-24/25.**~~ **Done, all Codex-reviewed clean** (review rule switched
   from Fable to Codex — see CLAUDE.md; UI subagent gate also lifted, see ui-spec.md): tier-0
   Unranked icon fix, raw presences debug dump, hold-last-match table (Menus keeps the last
   table + "Last match" chip), and the KD column (same 3-match window as HS%, zero extra
   requests, `PlayerRow.kd`). Party grouping diagnosed as working-as-possible pending a
   party-lobby capture — full findings in roadmap.md. All uncommitted as of the session end.
8. ~~**Audit fix batches.**~~ **Done 2026-08-25.** The Codex mega audit's findings (6 high /
   8 medium / 7 low) were all fixed in three implementation batches + a frontend/config pass,
   each Codex-reviewed (three review rounds on batch 1 alone). Highlights: lockfile-staleness
   reconnect, enrichment finality with bounded per-player retries, full 429/auth coverage with
   Retry-After, malformed-payload rejection with privacy-safe defaults, static-cache
   validation + cooldown top-up, per-match payload cache (30->3 downloads per overlapping
   lobby), in-game glz halved, sub-second cancellation, friend-presence pokes eliminated,
   bounded caches, CSP enabled, image retry. The audit file was deleted after completion;
   the only deferred items live in roadmap.md (release-workflow validation, CSP smoke test).
   Two false positives recorded there too (hold-last-match, React 18). Release hold lifted
   pending the auto-updater (see roadmap "Last").
9. ~~**Feature session 2026-08-25 (second).**~~ **Done, all Codex-reviewed clean** (one high
   finding — terminal-error giveups left cells pending forever — fixed and re-reviewed):
   incremental stat loading (per-row `pending` flags, coalesced settle-point emission,
   `enriched` reworked to "all stats settled"; contract in `docs/ipc-contract.md`), chroma
   skin icons (`CHROMA_SOCKET_ID` pending live verification; static-cache schema guard),
   HS%/KD window 3 → 5 (burst 41 → 61 requests worst case), peak-act label current-act
   fallback, and the updater UI (header `VersionBadge` fed by a build-time `__APP_VERSION__`
   define; `checkForUpdates()` stub in `src/lib/updater.ts` is the auto-updater seam). CSP smoke test dev half passed live (release-shaped `--debug` exe check still
   pending). **Session closed with everything uncommitted (user choice); live verification
   of the new features deferred — user will report anything broken in a later session.** A
   `--debug` exe for the CSP check + live look sits at
   `src-tauri/target/debug/valorant-lightweight-tracker.exe`. Known pre-existing flaky test:
   `a_rate_limit_deadline_outlives_the_request_that_earned_it` (Windows Instant granularity).
10. ~~**Dev logging session 2026-08-26.**~~ **Done, committed (`feat(debug)`), Codex-reviewed**
   (one high finding — full puuids/match ids in `net` log paths — fixed, re-reviewed clean):
   debug-build-only live console logging via `vlt_log!` in `src-tauri/src/debug_log.rs`
   (release builds compile it out entirely; zero new deps). Categories: `state` (publish
   transitions/updates incl. pending-row counts), `rebuild` (poke vs pregame tick), `net`
   (per-request seq/status/latency, 429 gate events; ids truncated to 8 chars, query dropped),
   `enrich` (phase summaries, chroma/loadout counts, stats-cache hits), `ws`, `conn`. See
   `docs/testing.md` "Live console log". Purpose: live-verify the item-11 open questions
   (chroma socket id, incremental fill-in, rate-limit burst, pregame updates) from the
   terminal. Same session also investigated the pregame poll: it costs **2 remote GLZ
   requests/sec** (match-id lookup + match fetch every tick, cache unused in pregame);
   "all 5 **locked**" is a safe stop condition ("selected" is not), and the own-presence
   websocket poke catches pregame→ingame independently of the tick — not yet implemented,
   see the todo list below.
11. **Session todo list (2026-08-26, not yet implemented):**
   - **Version/update UI redesign** — user dislikes the current header `v0.1.0` text next to
     "6m ago"; rework `VersionBadge` presentation and decide the future update-available look
     (auto-updater seam). Pending user input: tucked away vs visible-but-restyled.
   - **Pregame poll: stop after all 5 allies locked** (safe per investigation above).
     Footnote from the 2026-08-26 log analysis: while touching this code, also suppress the
     duplicated immediate 404 retry inside each transition backoff cycle (saves ~6 requests
     per pregame→ingame race; too small to stand alone).
   - **Pregame poll: cache the match id** — it can't change mid-pregame; halves poll cost.
     Log-quantified 2026-08-26: pregame ticks were 90 of 297 requests (30%) in a real run
     (`~/Downloads/log.txt`, keep as before/after benchmark).
   - **Unknown queue id leaks raw into the UI** — a real run showed `mode=fortcollins`
     (`game_mode_name` in `src-tauri/src/riot/constants.rs` falls through to the raw queue
     id, and that string reaches the UI). Add the mapping or a friendlier fallback.
   - **Bounded-concurrency match-details fetches** — enrichment fetches match details
     sequentially with a 120ms delay each; the first 45-call burst took 23s wall-clock.
     Two in flight (keeping the 429 gate) could roughly halve initial enrichment time.
     **Gated on** first live-verifying the comp-match rate-limit burst (open item below) —
     the sequential pacing is deliberate 429 caution and no run has exercised a 429 yet.
   - **Doc fix**: `docs/backend-spec.md` "Pregame poll tick" section says "one local pregame
     GET per second"; actually two remote GLZ requests.
12. **Open items for later sessions** (user finishes the roadmap there):
   - **App screenshots** (deferred by user to a later stage): take from the debug-snapshot
     browser view; README has a commented placeholder at `docs/assets/app-screenshot.png`.
     Consider anonymizing names in the JSON first.
   - **First release**: deferred to last (user decision 2026-08-24) — auto-updater must be
     built before any public release. Order: features + polish first, then auto-updater,
     then `pnpm bump 0.1.0` -> commit -> tag `v0.1.0` -> push tag; verify the workflow's
     first run on GitHub (never exercised on a real runner).
   - **Still needing live verification**: party dot colours (needs a party lobby — every
     capture so far was solo; if they turn out broken, decide then: fix or strip the dot
     code), `latam`/`br` shard mapping (needs such an account), flat
     presence shape, pregame-vs-coregame match-id equality (cache upgrade path), the
     full-lobby 61-request stat burst under rate limits in a comp match (captures were
     Spike Rush), the chroma socket uuid (`CHROMA_SOCKET_ID`), and the incremental
     fill-in behavior in a live lobby.
   - Possible polish noted from the first real-data screenshot: quiet the "Default"/"Default"
     skin text pairs; heavy N/A texture on no-comp-history rows.

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
- Working on the UI without a live match, or capturing real snapshots to replay: see
  `docs/testing.md` (dev-only, absent from release builds).

Note: `cargo`/`rustc` are not on PATH by default in this environment; prepend the rustup
cargo bin dir first (PowerShell: `$env:Path = "$env:USERPROFILE\scoop\apps\rustup\current\.cargo\bin;$env:Path"`).

## Conventions

- Python work: always in a venv. Node work: pnpm.
- Keep docs/ updated whenever decisions or scope change — this folder is the cross-session memory.
