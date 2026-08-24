# UI Spec (v1 baseline)

Last updated: 2026-08-24. Status: baseline agreed in conversation; no reference images yet — user may supply some later.

> **PROCESS GATE:** Claude does NOT implement or create UI, and does NOT spawn UI subagents. When UI implementation/creation work comes up, Claude writes a complete, ready-to-paste agent prompt (spec, file paths, constraints, verification steps) and hands it to the user — the user runs it themselves with their preferred model. Small mechanical frontend fixes on existing UI (typo, broken import) are fine; anything design/implementation-level goes through the prompt handoff.

## Core principle: images over text

The user's main gripe with vRY's TUI is text ranks. Every place an image exists on valorant-api.com, use it instead of text:

- Rank → tier icon (`/v1/competitivetiers`, `largeIcon`/`smallIcon`) — icon primary, name as tooltip/secondary
- Agent → portrait (`/v1/agents`, `displayIcon`)
- Map → splash/list-view image (`/v1/maps`)
- Cache these locally, keyed by game version (`/v1/version`), refresh on version change.

## Layout

- Single window, single screen. No tabs/navigation in v1. The player table IS the app.
- **Header strip:** map name + mode, app-state chip ("Waiting for match" / "Agent select" / "In match") that doubles as the health indicator, subtle "last updated" text.
- **Main region:** table split into two team blocks (ally / enemy), subtle teal/red tinted accents — not loud fills.
- **Row, left→right:** agent portrait · name#tag · current rank icon (+RR) · peak rank icon (smaller, muted) · account level. Party members marked with matching dot color or thin bracket.

## Style

- Dark theme only for v1.
- Valorant-adjacent, not a skin clone: near-black background, one red/coral accent, whitespace, let the rank/agent imagery carry the visual weight. Restraint > decoration.
- The empty/waiting state gets real design attention — it's the most-seen screen.
- Stack: React + Tailwind v4 (already scaffolded).

## Behavior

- Websocket-driven auto-refresh on game-state change; no manual refresh button.
- Click a row → copy `name#tag` to clipboard.
- Normal resizable window for v1. Compact always-on-top overlay mode is a possible later feature — don't build, don't block.

## Open items

- User may still provide reference images; revisit style section when they do.
- Score display in header: later, once backend exposes it.
