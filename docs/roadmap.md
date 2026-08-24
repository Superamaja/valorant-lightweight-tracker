# Roadmap

Later work, in rough order. Items move out of here and into project-context.md when they become active.

## Near-term (after phase-1 backend + review)

- **Live API verification** — user launches Valorant; probe endpoints live, confirm phase-1 data and exact field availability for the tier-2 columns (see ui-spec.md wishlist).
- **Phase-2 backend** — HS%, WR, RR delta per game, last-5 W/L (pd match-history + competitiveupdates), equipped Vandal skin (coregame loadouts). Needs throttling + per-player caching for rate limits.
- **UI implementation** — via user-run agent; Claude delivers the prompt (see gate in ui-spec.md). Build against docs/ipc-contract.md.

## Done

- **Release pipeline (GitHub Actions → Releases page)** — ✅ built (2026-08-24). Spec: `docs/release.md`; workflow: `.github/workflows/release.yml`.
  - **Portable single exe only** — no installer. Tag push (`v*`) on `windows-latest` runs `pnpm tauri build --no-bundle` and `softprops/action-gh-release@v2` attaches `valorant-lightweight-tracker-v<version>.exe` to a GitHub Release (auto-generated notes). No NSIS/MSI, no updater json, no signing.
  - **No auto-updater** (maybe future); updates are manual downloads.
  - Version sync: `pnpm bump <x.y.z|patch|minor|major>` (`scripts/bump-version.mjs`, zero deps) keeps package.json + Cargo.toml + tauri.conf.json identical; refuses if they disagree.
  - Unsigned exe → SmartScreen; the "More info → Run anyway" note is in the README.
  - Usage: `pnpm bump …` → commit → `git tag vX.Y.Z` → push tag → Actions builds the Release. Full checklist + first-time notes (repo on GitHub, Actions enabled) in `docs/release.md`.

## Later

- **Discord RPC** — architecture seam exists; not built (user decision).
- **vRY upstream check** — user-triggered every few weeks; procedure + last-checked hash live in docs/maintenance.md.

## Rejected

- **Overlay mode** — user decision: stays a separate app window, never an overlay. (Injected overlays are Vanguard-risky; a separate window is safe, and we're not doing always-on-top either.)
