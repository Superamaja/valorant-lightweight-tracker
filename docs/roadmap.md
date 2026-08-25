# Roadmap

Later work, in rough order. Items move out of here and into project-context.md when they become active.

## Near-term (after phase-1 backend + review)

- **Live API verification** — user launches Valorant; probe endpoints live, confirm phase-1 data and exact field availability for the tier-2 columns (see ui-spec.md wishlist).
- **Phase-2 backend** — HS%, WR, RR delta per game, last-5 W/L (pd match-history + competitiveupdates), equipped Vandal skin (coregame loadouts). Needs throttling + per-player caching for rate limits.
- **UI implementation** — via user-run agent; Claude delivers the prompt (see gate in ui-spec.md). Build against docs/ipc-contract.md.

## Done

- **KD column** — ✅ built + Codex-reviewed clean (2026-08-25). Ratio over the same 3-match window as HS%, from match-details payloads already fetched (zero extra requests). `PlayerRow.kd`, 2 dp, deaths 0 -> kills, null = no data; column after HS%. Verified live in a real match.
- **Table spacing rebalance** — ✅ (2026-08-25, after live-screenshot feedback). Slack spreads via fr shares (Player 3fr, others 1fr); skin art capped at 88px (`SKIN_ART_WIDTH`) with the phantom track widened to 108px so phantom art -> rank icon gets ~20px at 1000px (was 10px); name-cell copy button is an absolute opaque chip (fixed premature self-name truncation + a Codex overlay finding). Ingame row minimum 946px; Pregame 722px, unaffected. The next fixed-width column has ~50px of headroom at 1000px.
- **Hold last match table** — ✅ built + reviewed (2026-08-24/25). Menus after a match keeps the last table with a muted "Last match" header chip; ValorantNotRunning/error unchanged. Frontend-only (App.tsx ref-in-effect + Header chip), one-commit revert.
- **Unranked (tier 0) icon** — ✅ fixed (2026-08-24): `static_data.rs` no longer short-circuits tier 0; the competitivetiers Unranked icon flows to current + peak rank. Falls back to no-icon only when the table lacks tier 0.
- **Raw presences debug dump** — ✅ built (2026-08-24): `VLT_DEBUG_CAPTURE` also writes `presences-{n:04}.json` (raw `/chat/v4/presences` body) per rebuild; shared counter with snapshots. See docs/testing.md.

- **Release pipeline (GitHub Actions → Releases page)** — ✅ built (2026-08-24). Spec: `docs/release.md`; workflow: `.github/workflows/release.yml`.
  - **Portable single exe only** — no installer. Tag push (`v*`) on `windows-latest` runs `pnpm tauri build --no-bundle` and `softprops/action-gh-release@v2` attaches `valorant-lightweight-tracker-v<version>.exe` to a GitHub Release (auto-generated notes). No NSIS/MSI, no updater json, no signing.
  - **No auto-updater yet** — now decided as required before public release; see "Last" section below.
  - Version sync: `pnpm bump <x.y.z|patch|minor|major>` (`scripts/bump-version.mjs`, zero deps) keeps package.json + Cargo.toml + tauri.conf.json identical; refuses if they disagree.
  - Unsigned exe → SmartScreen; the "More info → Run anyway" note is in the README.
  - Usage: `pnpm bump …` → commit → `git tag vX.Y.Z` → push tag → Actions builds the Release. Full checklist + first-time notes (repo on GitHub, Actions enabled) in `docs/release.md`.

## Last (deliberately after all features + polish)

- **Auto-updater** — user decision (2026-08-24): required before going public, but built last. Until then, no public release push; the existing pipeline stays for private/test tags only.
- **First public release** — `pnpm bump 0.1.0` → tag → push; verify the never-exercised workflow on a real runner. Happens only after features, polish, and the auto-updater are done.

## Later

- **Party grouping — awaiting party-lobby evidence.** Diagnosis so far (2026-08-24/25): vRY has no extra mechanism — same `/chat/v4/presences` intersection we do (its "wait_for_presence" is a no-op bug). Decoded live dumps prove Riot only pushes presence for self + friends: mid-game the roster held 0 of the 9 other match players, and no field enumerates party members' puuids. Our `party_grouping()` returning empty was CORRECT for a solo queue with no friends in the lobby — not a bug. Open question: does Riot push presence for your own party members (likely — would make party dots work for premades + friends, which is plausibly all vRY ever shows)? **Test: queue one game in a party with `VLT_DEBUG_CAPTURE` on**, check the dumps for the partymate + shared partyId; also check the pregame-phase dumps. Then either implement a bounded pregame re-poll (~1-2s up to ~10s) if data appears late, or close the item as working-as-possible. Cheap extra: decoder ignores `isPartyOwner` (party-leader marker, free to add).
- **Header score display** — live round score is already in the data we fetch: own presence `partyOwnerMatchScoreAllyTeam` / `partyOwnerMatchScoreEnemyTeam`. Nearly free to expose in the snapshot + header.
- **Discord RPC** — architecture seam exists; not built (user decision).
- **vRY upstream check** — user-triggered every few weeks; procedure + last-checked hash live in docs/maintenance.md.

## Rejected

- **Overlay mode** — user decision: stays a separate app window, never an overlay. (Injected overlays are Vanguard-risky; a separate window is safe, and we're not doing always-on-top either.)
