# Maintenance

## vRY upstream check

The user periodically starts a session and asks to "check vRY since last commit" (or similar). Procedure:

1. Read the hash below. Diff upstream from it: `https://github.com/mdevio/VALORANT-rank-yoinker/compare/<last-checked>...main` (fetchable via WebFetch or the GitHub API).
2. Look for changes to endpoint URLs, auth/headers, presence JSON shape, MMR/rank parsing, or new pitfall handling — ignore console-UI/config/feature churn.
3. If something relevant changed: update docs/backend-spec.md and the Rust backend accordingly (Fable-review + verify + commit per CLAUDE.md rules).
4. Whether or not anything changed, update the hash + date below and commit.

**Last checked vRY commit:** `0e30d916d366ecff6433ff6e95f69fee93a3608c` (main as of 2026-08-21; checked 2026-08-24 — spec originally derived from this commit)

## Static data cache

valorant-api.com data is keyed to the game version (`/v1/version`); the backend refreshes its cache when the version changes. No manual action normally needed.
