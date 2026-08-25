# Valorant Lightweight Tracker

Lightweight desktop app showing an in-match player table (ranks, RR, peak rank, level, agent) for the user's current Valorant match. Windows-first.

## Session memory

`docs/` is the project's persistent memory. Read `docs/project-context.md` at the start of every session before doing anything. When a decision is made, scope changes, or important info is learned, update the relevant file in `docs/` in the same session — do not let it go stale.

## Docs index

- `docs/project-context.md` — decisions made, scope, current status, next steps. **Start here.**
- `docs/data-sources.md` — where match/player data comes from and reference material.
- `docs/ui-spec.md` — agreed UI baseline. Contains a process gate: Claude never implements UI or spawns UI subagents — it writes a ready-to-paste agent prompt and the user runs it themselves (their choice of model + higher effort).
- `docs/roadmap.md` — later work (release pipeline: portable single exe via GitHub Actions, no auto-updater yet) and rejected ideas.
- `docs/maintenance.md` — vRY upstream check procedure + last-checked commit hash. Use when the user asks to "check vRY" / "maintain".

## Hard rules

- vRY (https://github.com/mdevio/VALORANT-rank-yoinker) is the **reference implementation** for data correctness. When our numbers disagree with vRY, vRY is right until proven otherwise.
- Do NOT reuse logic from ValoTracker (https://github.com/Londopy/ValoTracker) — user verified its data is wrong.
- Scope: in-match player table only. Discord RPC is a possible future feature — keep the architecture open to it, but do not build it.
- README.md must NEVER contain em-dashes. Rewrite the sentence or use other punctuation (colons, commas, periods, parentheses).
- Commit messages must never mention Claude, model names (Fable/Opus/Sonnet), or session links.
- Semantic commits use scopes: `type(scope): summary`. Scopes: `backend` (Rust/Riot pipeline), `ui` (React frontend), `debug` (capture/testing tooling), `release` (pipeline/versioning), `docs`. Example: `fix(backend): poll pregame roster`. Adopted 2026-08-25; older commits are unscoped, leave them.
- After every implementation pass (agent or inline), run a **Codex code review** (per the global CLAUDE.md: `--model gpt-5.6-sol --effort high`, read-only, with an explicit risk threshold + stop condition) before the work is considered done. Review targets: correctness vs docs/backend-spec.md, code quality (DRY, SoC, naming), and lightweightness (no unnecessary dependencies, no wasted allocations/requests — this app's whole point is being lightweight). (Replaced the earlier Fable-review rule, 2026-08-24.)
