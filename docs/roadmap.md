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

- **Release workflow** (do during release prep): validate tag format (`vX.Y.Z`, matching the manifests) and run `cargo test` before publishing; any `v*` tag currently publishes. Also (user decision 2026-08-25): the published exe filename must NOT contain the version — keep it stable across releases so the auto-updater can replace the file in place; the version lives in the tag/Release name only.
- **Updater UI** — built 2026-08-25 (header version badge + check-for-updates affordance; see ui-spec "Version / updater affordance"). Remaining: replace the stub body of `checkForUpdates()` in `src/lib/updater.ts` with the real auto-updater when it's built — the UI needs no further changes.
- **Auto-updater** — user decision (2026-08-24): required before going public, but built last. Until then, no public release push; the existing pipeline stays for private/test tags only.
- **First public release** — `pnpm bump 0.1.0` → tag → push; verify the never-exercised workflow on a real runner. Happens only after features, polish, and the auto-updater are done.

## Later

- **Header score display** — live round score is already in the data we fetch: own presence `partyOwnerMatchScoreAllyTeam` / `partyOwnerMatchScoreEnemyTeam`. Nearly free to expose in the snapshot + header.
- **Discord RPC** — architecture seam exists; not built (user decision).
- **`rustfmt.toml` guard** — the repo is deliberately not rustfmt-clean (wider hand-maintained style); a stray `cargo fmt` once reformatted 16 files and had to be hand-reverted. Add a `rustfmt.toml` matching the house style (or an explicit "do not run cargo fmt" note) so it cannot happen again.
- **vRY upstream check** — user-triggered every few weeks; procedure + last-checked hash live in docs/maintenance.md.

## Rejected

- **Party grouping fix** — dropped by user decision (2026-08-25), no need. Diagnosis (kept for the record): vRY has no extra mechanism — same `/chat/v4/presences` intersection we already do; Riot only pushes presence for self + friends (+ likely own partymates), so arbitrary-player party detection is impossible from this data. Our `party_grouping()` returning empty in a friendless solo queue was correct behavior. The existing party-dot rendering stays as-is: it lights up when the data exists (premades/friends), silently shows nothing otherwise.

- **Overlay mode** — user decision: stays a separate app window, never an overlay. (Injected overlays are Vanguard-risky; a separate window is safe, and we're not doing always-on-top either.)
