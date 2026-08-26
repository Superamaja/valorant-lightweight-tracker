# Roadmap

Later work, in rough order. Items move out of here and into project-context.md when they become active.

## Deferred from the 2026-08-25 mega audit (everything else was fixed; audit file deleted)

- **CSP runtime smoke test** — dev run passed 2026-08-25 (live match visuals rendered, console clean); still pending: the release-shaped check. A `--debug` exe (release `csp` + devtools) is already built at `src-tauri/target/debug/valorant-lightweight-tracker.exe`; checklist: Ctrl+Shift+I console shows no CSP violations, valorant-api images render, table updates live.

## Done (compact history)

- **2026-08-25 feature session** — all Codex-reviewed clean: incremental stat loading (per-row `PlayerRow.pending` flags, snapshots emitted as stats settle with 250 ms coalescing, `enriched` now means "all stats settled", terminal-error giveups settle the table instead of leaving skeletons); chroma-accurate skin icons (loadout chroma socket + static-cache chroma art, base-icon fallback, cache schema guard — socket uuid pending live verification); HS%/KD window widened 3 → 5 matches (worst-case stat burst 41 → 61 requests, match-details cache cap 128); peak-act label falls back to the current act instead of showing nothing. CSP smoke test dev half passed live.
- **2026-08-25 hardening + efficiency batches** — all six audit highs fixed (lockfile staleness, enrichment finality, 429/auth coverage incl. Retry-After, rebuild retries, malformed-payload rejection with safe privacy defaults, static-cache validation with cooldown top-up), Codex-reviewed through three passes. Request-efficiency batch: per-match payload cache, redundant in-game glz skip, prompt cancellation.
- **2026-08-25 features** — pregame 1s poll for live agent picks (verified live); KD column (3-match window, zero extra requests, verified live); hold-last-match table + chip (verified live); table spacing rebalance; Unranked tier-0 icon fix; raw presences debug dump.
- **2026-08-24 release pipeline** — tag push `v*` → Actions builds portable exe onto a GitHub Release; `pnpm bump` syncs the three version files; no installer, no signing, no auto-updater yet. Details: `docs/release.md`.
- Earlier milestones (backend phases 1–2, UI build + revisions, debug/testing mode) are recorded in `docs/project-context.md`.

## Last (deliberately after all features + polish)

- ~~**Release workflow**~~ **Done 2026-08-26** (Codex-reviewed): strict `vX.Y.Z` tag gate, tag-vs-manifest verification, `pnpm build` + `cargo test --locked` gates, stable version-free exe asset name; `pnpm bump` keeps `Cargo.lock` in sync so the `--locked` gate survives a bump. Details: `docs/release.md`. Still never run on a real runner.
- ~~**Updater UI**~~ **Done 2026-08-25**, wired to the backend 2026-08-26 (header version badge + check-for-updates affordance; see ui-spec "Version / updater affordance"). The badge is now the install button.
- ~~**Auto-updater**~~ **Done 2026-08-26**: `check_update` / `apply_update` in `src-tauri/src/updater.rs` — GitHub `releases/latest`, SHA-256-verified download, Windows rename swap, detached relaunch, leftover cleanup on start. Requires a public repo. Details: `docs/release.md`.
- ~~**First public release**~~ **Done 2026-08-26**: `v0.1.0` published — the workflow's first real run went green end to end (all gates, exe build, checksum, Release with both assets: exe 11.5 MB + 64-byte `.sha256`). Run: https://github.com/Superamaja/valorant-lightweight-tracker/actions/runs/32953164443. The auto-updater's end-to-end swap is still unexercised — the next release (v0.1.1) is its first real test. Workflow actions bumped off the deprecated Node 20 runtime 2026-08-26 (checkout@v7, setup-node@v7, pnpm/action-setup@v6, action-gh-release@v3).

## Later

- **Header score display** — live round score is already in the data we fetch: own presence `partyOwnerMatchScoreAllyTeam` / `partyOwnerMatchScoreEnemyTeam`. Nearly free to expose in the snapshot + header.
- **Discord RPC** — architecture seam exists; not built (user decision).
- **`rustfmt.toml` guard** — the repo is deliberately not rustfmt-clean (wider hand-maintained style); a stray `cargo fmt` once reformatted 16 files and had to be hand-reverted. Add a `rustfmt.toml` matching the house style (or an explicit "do not run cargo fmt" note) so it cannot happen again.
- **vRY upstream check** — user-triggered every few weeks; procedure + last-checked hash live in docs/maintenance.md.

## Rejected

- **Party grouping fix** — dropped by user decision (2026-08-25), no need. Diagnosis (kept for the record): vRY has no extra mechanism — same `/chat/v4/presences` intersection we already do; Riot only pushes presence for self + friends (+ likely own partymates), so arbitrary-player party detection is impossible from this data. Our `party_grouping()` returning empty in a friendless solo queue was correct behavior. The existing party-dot rendering stays as-is: it lights up when the data exists (premades/friends), silently shows nothing otherwise.

- **Overlay mode** — user decision: stays a separate app window, never an overlay. (Injected overlays are Vanguard-risky; a separate window is safe, and we're not doing always-on-top either.)
