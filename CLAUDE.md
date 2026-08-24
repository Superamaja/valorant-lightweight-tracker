# Valorant Lightweight Tracker

Lightweight desktop app showing an in-match player table (ranks, RR, peak rank, level, agent) for the user's current Valorant match. Windows-first.

## Session memory

`docs/` is the project's persistent memory. Read `docs/project-context.md` at the start of every session before doing anything. When a decision is made, scope changes, or important info is learned, update the relevant file in `docs/` in the same session — do not let it go stale.

## Docs index

- `docs/project-context.md` — decisions made, scope, current status, next steps. **Start here.**
- `docs/data-sources.md` — where match/player data comes from and reference material.
- `docs/ui-spec.md` — agreed UI baseline. Contains a process gate: Claude never implements UI or spawns UI subagents — it writes a ready-to-paste agent prompt and the user runs it with their own model.

## Hard rules

- vRY (https://github.com/mdevio/VALORANT-rank-yoinker) is the **reference implementation** for data correctness. When our numbers disagree with vRY, vRY is right until proven otherwise.
- Do NOT reuse logic from ValoTracker (https://github.com/Londopy/ValoTracker) — user verified its data is wrong.
- Scope: in-match player table only. Discord RPC is a possible future feature — keep the architecture open to it, but do not build it.
