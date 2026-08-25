# Roadmap

Later work, in rough order. Items move out of here and into project-context.md when they become active.

## Deferred from the 2026-08-25 mega audit (everything else was fixed; audit file deleted)

- **Release workflow** (do during release prep): validate tag format (`vX.Y.Z`, matching the manifests) and run `cargo test` before publishing; any `v*` tag currently publishes.
- **CSP runtime smoke test** — CSP was enabled 2026-08-25 (`tauri.conf.json`: self + media.valorant-api.com images, ipc.localhost connect-src, devCsp adds ws: for HMR) and `pnpm build` passes, but a live check is pending: confirm images render, the table populates, and devtools shows no CSP violations in both `pnpm tauri dev` and a release exe.

## Done (compact history)

- **2026-08-25 hardening + efficiency batches** — all six audit highs fixed (lockfile staleness, enrichment finality, 429/auth coverage incl. Retry-After, rebuild retries, malformed-payload rejection with safe privacy defaults, static-cache validation with cooldown top-up), Codex-reviewed through three passes. Request-efficiency batch: per-match payload cache, redundant in-game glz skip, prompt cancellation.
- **2026-08-25 features** — pregame 1s poll for live agent picks (verified live); KD column (3-match window, zero extra requests, verified live); hold-last-match table + chip (verified live); table spacing rebalance; Unranked tier-0 icon fix; raw presences debug dump.
- **2026-08-24 release pipeline** — tag push `v*` → Actions builds portable exe onto a GitHub Release; `pnpm bump` syncs the three version files; no installer, no signing, no auto-updater yet. Details: `docs/release.md`.
- Earlier milestones (backend phases 1–2, UI build + revisions, debug/testing mode) are recorded in `docs/project-context.md`.

## Last (deliberately after all features + polish)

- **Auto-updater** — user decision (2026-08-24): required before going public, but built last. Until then, no public release push; the existing pipeline stays for private/test tags only.
- **First public release** — `pnpm bump 0.1.0` → tag → push; verify the never-exercised workflow on a real runner. Happens only after features, polish, and the auto-updater are done.

## Later

- **Header score display** — live round score is already in the data we fetch: own presence `partyOwnerMatchScoreAllyTeam` / `partyOwnerMatchScoreEnemyTeam`. Nearly free to expose in the snapshot + header.
- **Chroma-accurate skin icons** (noticed 2026-08-25) — the coregame loadout payload carries a skin-chroma socket alongside the skin id, and valorant-api exposes per-chroma art (`chromas[].fullRender`/`displayIcon`). Read the chroma socket in `loadout.rs`, extend the static skin cache with chroma→icon, fall back to base skin icon when chroma art is missing. Small change, zero extra requests.
- **Discord RPC** — architecture seam exists; not built (user decision).
- **vRY upstream check** — user-triggered every few weeks; procedure + last-checked hash live in docs/maintenance.md.

## Rejected

- **Party grouping fix** — dropped by user decision (2026-08-25), no need. Diagnosis (kept for the record): vRY has no extra mechanism — same `/chat/v4/presences` intersection we already do; Riot only pushes presence for self + friends (+ likely own partymates), so arbitrary-player party detection is impossible from this data. Our `party_grouping()` returning empty in a friendless solo queue was correct behavior. The existing party-dot rendering stays as-is: it lights up when the data exists (premades/friends), silently shows nothing otherwise.

- **Overlay mode** — user decision: stays a separate app window, never an overlay. (Injected overlays are Vanguard-risky; a separate window is safe, and we're not doing always-on-top either.)
