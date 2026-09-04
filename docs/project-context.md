# Project Context

Last updated: 2026-09-04

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
| License | PolyForm Noncommercial 1.0.0 | User wants the work protected from commercial copying; not OSI open source, advertise as "source-available / free for personal use" |

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
11a. ~~**Todo execution 2026-08-26.**~~ **Done, all Codex-reviewed, committed per change:**
   the UI polish batch (skin columns after ΔRR; rank h-8 / peak h-6; amber `YOU`; age hidden
   while live; unranked icon without dash; party bar instead of dot; row plate settled at
   `to-white/[0.025]` after a live A/B — 0.015 was invisible, 0.04 too loud; toggle kept in
   `src/lib/table.ts`). One review finding triaged out (NaN age guard — dev-only input,
   pre-existing behavior). The pregame backend batch (tick pauses once the roster is fully
   locked — one high review finding fixed: the flag is now cleared at the start of every
   rebuild so failure paths keep the recovery tick; pregame match id cached, steady-state
   tick 2 → 1 request/s; 404 immediate-retry dedup; backend-spec corrected to "remote glz").
   The Retake fix (`fortcollins` → "Retake"; unknown queue ids now title-cased + dev-logged
   instead of leaking raw). vRY cross-check skipped (user: no recent upstream commits).
   Screenshot workflow note: browser A/B captures should be taken at the app's real
   1000×700 (constrain the page and capture that region; the Chrome window resize tool
   can silently fail on a maximized window).
11b. ~~**Version/update + last-match rework (2026-08-26, second pass).**~~ **Done,
   Codex-reviewed, committed:** user picked an A+B combo — the version line + update check
   now live on the waiting screen (`StatusScreen`), the header carries no version text and
   shows an `Update: vX.Y.Z` chip only when a check finds one (shared store in
   `src/lib/updater.ts` + `useUpdateState`; `checkForUpdates` stub still the auto-updater
   seam). Menus reworked: waiting screen by default, held table behind a "View last match"
   toggle that resets whenever a live match starts (single-snapshot hold unchanged — no
   memory growth). Follow-up micro-pass from live use: locked agent picks no longer get the
   accent-red ring (pregame is allies-only; locked now renders like any normal portrait) and
   the waiting title went 15px → 18px. README: held-table + party bullets replaced with the
   tracker.gg link-out bullet.
11c. **Handoff to the next session (written 2026-08-26, end of session):**
   - The user is playing a **competitive match right now** with the fresh debug exe and will
     provide the console log next session. That log decides: the rate-limit burst
     verification (any `429: backoff armed` lines?), the bounded-concurrency gate below,
     and live confirmation of the locked-roster tick pause + pregame match-id cache
     (steady-state ticks should show one request, not two). If they queued with a friend,
     also check the party bars; the in-match devtools console (Ctrl+Shift+I) doubles as the
     pending release-shaped CSP smoke test.
   - Next work, per user: the roadmap "Last" items (release workflow hardening →
     auto-updater → first release). **Nothing has been started** — a workflow-hardening
     implementation and an auto-updater planning pass were both cancelled by the user
     before producing anything; begin fresh next session.
   - Final waiting-screen sizes after live iteration: title 18px, subtitle 13px, action
     button 12px, version line 10px (deliberately quiet). Locked agent picks render with
     the normal portrait ring (accent ring removed; pregame is allies-only).
11d. ~~**Comp-log session 2026-08-26 (third pass).**~~ **Done, Codex-reviewed (two HIGH
   findings fixed + re-reviewed clean), committed per change.** The user's live comp-match
   debug log verified: zero 429s across the full session (~137 requests, both stat bursts),
   locked-roster tick pause + pregame match-id cache (one request per steady-state tick),
   chroma sockets (10/10 vandal+phantom), incremental fill-in (pending_rows stepped to 0),
   and pregame/coregame match-id equality (same id both phases). Work shipped:
   - Peak rank icon restored to the current-rank size (user request; shared `ICON` again).
   - **Two-lane match-details fetching** (was gated on the 429 verification, now unblocked):
     `plan_window` dedupes ids up front, chunks of 2 through the same 429 gate, 120ms per
     dispatch (~60ms/request effective). Review fix: `await_rate_limit` loop re-checks the
     gate after every sleep so a deadline extended by the other lane is honored.
   - **Presence-gap grace**: the log caught a mid-match `Ingame -> ValorantNotRunning`
     flash (4s, transient local presence gap). `Session.not_ready_streak` now suppresses
     the first 2 consecutive NotReady publishes while a live match is on screen (~3s grace
     via the existing retry loop); new debug `presence` log category lights the three
     formerly-silent NotReady causes.
   - **RateLimitGate boundary fix**: an exactly-reached deadline reported `Some(0ns)` and
     stayed armed; now reports `None` and clears. This was the real cause of the "flaky"
     `a_rate_limit_deadline_outlives_the_request_that_earned_it` test — no longer flaky,
     which matters because `cargo test` now gates releases.
   - **Release workflow hardening** (first roadmap "Last" item): strict `vX.Y.Z` tag gate,
     tag-vs-manifest verification, `pnpm build` + `cargo test --locked` gates, stable
     version-free exe asset name. Review fix: `pnpm bump` now also rewrites the app's
     entry in `Cargo.lock` (four files, all in the consistency check) so the `--locked`
     gate survives a bump. `docs/release.md` updated.
   - **Auto-updater built** (`feat(release)`, Codex plan review "sound, no blockers" +
     code review: 1 high / 3 med / 1 low all fixed, re-reviewed clean): `check_update` /
     `apply_update` in `src-tauri/src/updater.rs` — GitHub releases/latest (unauthenticated,
     public repo required), strictly-newer version gate at install time, sha256-verified
     streamed download (sha2 was already in the lock via Tauri tooling — no new packages),
     checked rename-dance swap with rollback (distinct "stranded as .exe.old" error),
     detached relaunch, `.exe.old` cleanup on start. Workflow publishes the `.sha256`
     asset. UI: header chip installs (retry + `installError` on failure), check/install
     mutually exclusive. Contract in `docs/ipc-contract.md`; user flow in `docs/release.md`.
     **Not yet live-tested end-to-end — needs two real releases to exercise an actual
     update.**
11e. ~~**Release day (2026-08-26, fourth pass).**~~ **Done, committed + pushed:**
   - **v0.1.0 shipped.** First-ever real workflow run went green end to end (10m37s, all
     gates, both assets). Release: repo Releases page; run link in roadmap "Last".
   - **Changelog-driven release notes** (Codex-reviewed clean): `CHANGELOG.md` at the
     repo root, workflow gate fails a tag with no matching section, section published as
     the Release description (auto commit list appended). v0.1.0's release body was
     retro-filled. Convention: Claude does version bumps and writes the plain-English
     entry — recorded in CLAUDE.md (moved from local memory so it travels across
     machines, user request).
   - CLAUDE.md also gained the commit-after-every-change convention (same portability
     reason).
11f. ~~**Handoff (end of 2026-08-26 sessions).**~~ **v0.1.1 shipped 2026-08-26** (release
   tooling only, no app changes — purpose is to be the auto-updater's first end-to-end
   target): tag pushed, workflow run green end to end on the newly-bumped actions
   (checkout@v7, setup-node@v7, pnpm/action-setup@v6, action-gh-release@v3), both assets
   attached (exe 11.5 MB + `.sha256`). The `CHANGELOG.md` gate ran for real for the first
   time and passed; the v0.1.1 section became the Release description.
   - **Swap test verified 2026-08-26**: the user launched their v0.1.0 exe, it offered
     v0.1.1, installed via the header chip, swapped and relaunched cleanly (running
     instance restarted as v0.1.1).
   - Open verification items unchanged from item 12 + the two-lane fetch and presence
     grace next time the user plays with a debug build.
   - **Waiting-screen footer completed 2026-09-04**: `Report a bug` joined the version and
     `Copy diagnostics`, opening the repo's bug form with the version prefilled, and carrying
     the Simple Icons GitHub mark under the new verbatim-path icon rule.
11g. ~~**Diagnostics + bug-report session (2026-09-03/04).**~~ **Done, Codex-reviewed, v0.1.3
   tagged and pushed (workflow run green, see roadmap "Last").** Trigger: a user
   reported "not detecting my match" with no details and no way to file it (Issues were
   disabled). Shipped:
   - `.github/ISSUE_TEMPLATE/bug_report.yml` + `config.yml` (screen / version / region / mode
     / setup checkboxes / diagnostics paste); README "Troubleshooting" (one paragraph per
     waiting screen) + "Reporting a bug". **User must enable Issues in repo settings.**
   - Release-build diagnostics: bounded `Diagnostics` record on `TrackerState`
     (`src-tauri/src/diagnostics.rs`), written only at existing error/decision points (no
     per-request cost), rendered by `get_diagnostics` into a plain-text report (lockfile,
     local API, own presence incl. raw `sessionLoopState`, remote region/shard with the
     latam/br "inferred" note, websocket; 8-char ids, no secrets; golden + privacy tests).
     `debug_log::short` is now release code. `windows-version` added as a cfg(windows) dep
     (already in the lock via tao/wry, no new package).
   - UI: waiting-screen footer `v{version} · Copy diagnostics · Report a bug` (GitHub mark
     from Simple Icons), quiet header `Copy diagnostics` while a table shows; textarea
     fallback (out of flow, `field-sizing-content max-h-40 w-96`) only when the clipboard
     refuses. Fable visual review via Playwright in browser mode fixed a layout jump, a
     clipped textarea, and the default blue selection.
   - Icon sourcing rule added to CLAUDE.md + ui-spec "Icons".
   - Verification limits this session: the Linux cluster cannot link the Tauri crate
     (missing GTK/WebKit libs); Rust gates ran in a scratch crate that `#[path]`-includes
     the real sources against a tauri shim (198 tests, `cargo check --release` clean).
     Two **pre-existing** `clippy::nonminimal_bool` errors in `MatchCache::is_fresh_for`
     (`app_state.rs`, `!(ingame && !self.ingame)`) fail `-D warnings` with the current
     clippy; CI does not run clippy, fix next session.
   - **Still to live-verify on Windows**: clipboard write from a real release exe
     (WebView2), the `tauri::webview_version()` line, the Windows build line, and a real
     report from the affected user. `pnpm build` + browser mode verified here.
11. **Session todo list (2026-08-26, remaining):**
   - UI-review findings rejected by user (do not revisit): pip opacity/half-height rework,
     numeric right-alignment, empty-cell dash unification, last-match table dimming,
     self-row ring removal, outlier stat weighting.
12. **Open items for later sessions** (user finishes the roadmap there):
   - **First release**: deferred to last (user decision 2026-08-24) — auto-updater must be
     built before any public release. Order: features + polish first, then auto-updater,
     then `pnpm bump 0.1.0` -> commit -> tag `v0.1.0` -> push tag; verify the workflow's
     first run on GitHub (never exercised on a real runner).
   - **Still needing live verification** (comp-log session 2026-08-26 cleared the rest —
     see 11d): party dot colours (needs a party lobby — every capture so far was solo; if
     they turn out broken, decide then: fix or strip the dot code), `latam`/`br` shard
     mapping (needs such an account), flat presence shape, the two-lane fetch + presence
     grace in a live match (both new this session), and the release-shaped CSP smoke test
     (`--debug` exe, Ctrl+Shift+I in a match).
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
