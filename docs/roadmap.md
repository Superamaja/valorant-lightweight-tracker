# Roadmap

Later work, in rough order. Items move out of here and into project-context.md when they become active.

## Near-term (after phase-1 backend + review)

- **Live API verification** — user launches Valorant; probe endpoints live, confirm phase-1 data and exact field availability for the tier-2 columns (see ui-spec.md wishlist).
- **Phase-2 backend** — HS%, WR, RR delta per game, last-5 W/L (pd match-history + competitiveupdates), equipped Vandal skin (coregame loadouts). Needs throttling + per-player caching for rate limits.
- **UI implementation** — via user-run agent; Claude delivers the prompt (see gate in ui-spec.md). Build against docs/ipc-contract.md.

## Later

- **Release pipeline (GitHub Actions → Releases page)** — not yet specced. Decisions made (2026-08-24):
  - **Portable single exe only** — no installer. `tauri-apps/tauri-action` on tag push (`v*`) builds and attaches the exe to a GitHub Release.
  - **No auto-updater for now** (maybe future); updates are manual downloads.
  - Version bump must sync package.json + Cargo.toml + tauri.conf.json (one script, spec with the CI doc).
  - Unsigned exe → SmartScreen warning; document "More info → Run anyway" in README.
  - Spec the workflow properly (own doc) when we get here.
- **Discord RPC** — architecture seam exists; not built (user decision).
- **vRY upstream check** — user-triggered every few weeks; procedure + last-checked hash live in docs/maintenance.md.

## Rejected

- **Overlay mode** — user decision: stays a separate app window, never an overlay. (Injected overlays are Vanguard-risky; a separate window is safe, and we're not doing always-on-top either.)
