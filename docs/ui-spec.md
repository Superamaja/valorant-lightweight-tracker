# UI Spec (v1 baseline)

Last updated: 2026-08-24. Status: baseline agreed in conversation; no reference images yet — user may supply some later.

> **PROCESS GATE:** Claude does NOT implement or create UI, and does NOT spawn UI subagents. When UI implementation work comes up, Claude writes a complete, ready-to-paste agent prompt (spec, file paths, constraints, verification steps) and hands it to the user — the user runs it themselves with their preferred model and a higher effort level than subagents get. Small mechanical fixes to existing UI (typo, broken import) are fine; anything design/implementation-level goes through the prompt handoff. Fable review after the user's agent finishes still applies (user will ask for it).

## Core principle: images over text

The user's main gripe with vRY's TUI is text ranks. Every place an image exists on valorant-api.com, use it instead of text:

- Rank → tier icon (`/v1/competitivetiers`, `largeIcon`/`smallIcon`) — icon primary, name as tooltip/secondary
- Agent → portrait (`/v1/agents`, `displayIcon`)
- Map → splash/list-view image (`/v1/maps`)
- Cache these locally, keyed by game version (`/v1/version`), refresh on version change.

## Layout

- Single window, single screen. No tabs/navigation in v1. The player table IS the app.
- **Header strip:** map name + mode, app-state chip ("Waiting for match" / "Agent select" / "In match") that doubles as the health indicator, subtle "last updated" text.
- **Main region:** table split into two team blocks — **user's team ALWAYS first and ALWAYS blue** (enemy second, red), regardless of Riot's internal Red/Blue team ids (user dislikes vRY using raw API order/colors). The backend guarantees this ordering in players[] (ally block first, self first within it); the UI colors only by `isAlly`, never by the raw team id. Subtle tinted accents, not loud fills.
- **Row, left→right:** agent portrait · name#tag · current rank icon (+RR) · peak rank icon (smaller, muted) · account level. Party members marked with matching dot color or thin bracket.

## Style

- Dark theme only for v1.
- Valorant-adjacent, not a skin clone: near-black background, one red/coral accent, whitespace, let the rank/agent imagery carry the visual weight. Restraint > decoration.
- The empty/waiting state gets real design attention — it's the most-seen screen.
- Stack: React + Tailwind v4 (already scaffolded).

## Behavior

- Websocket-driven auto-refresh on game-state change; no manual refresh button.
- Click a player's name → open their tracker.gg profile in the default browser: `https://tracker.gg/valorant/profile/riot/{name}%23{tag}/overview` (Tauri opener plugin). Disabled for incognito/hidden players. Secondary action (right-click or small icon): copy `name#tag`.
- tracker.gg is link-out ONLY — their API has no Valorant support (exclusive Riot deal) and scraping violates their ToS. All stats stay self-calculated from Riot endpoints.
- Normal resizable window. Overlay mode is REJECTED (user decision) — this stays a separate app window permanently.

## Per-player data wishlist (agreed 2026-08-24)

Tier 1 = core pipeline (backend phase 1, in progress). Tier 2 = needs additional endpoints (backend phase 2). All confirmed possible — vRY or the documented API surface provides each.

| Column | Tier | Source |
|---|---|---|
| Party grouping | 1 | presence `partyId` — group players sharing an id, color-coded dots |
| Agent (portrait) | 1 | coregame `CharacterID` + valorant-api.com icon |
| Name#tag | 1 | name-service (respect incognito/hidden flag) |
| Current rank (icon) + RR | 1 | MMR endpoint |
| Peak rank (icon) | 1 | MMR seasonal data (tier-shift handling per backend spec) |
| Account level | 1 | `PlayerIdentity.AccountLevel` (respect hide flag) |
| HS% | 2 | pd match-history + match-details: headshot/total hit counts over recent matches |
| WR (win rate) | 2 | pd competitiveupdates / match-history: wins over recent comp matches |
| RR delta per previous game | 2 | pd competitiveupdates: `RankedRatingEarned` per match |
| Last 5 games W/L | 2 | same competitiveupdates data, rendered as 5 W/L pips |
| Vandal skin (maybe Phantom too) | 2 | coregame loadouts endpoint + valorant-api.com skin name/icon |

Tier 2 caveats: per-player history fetches multiply request count (10 players x N matches) — must be throttled/cached to avoid Riot rate limits (vRY hits this too); exact per-field availability to be confirmed against the live API once Valorant is running on this machine.

## Open items

- User may still provide reference images; revisit style section when they do.
- Score display in header: later, once backend exposes it.
