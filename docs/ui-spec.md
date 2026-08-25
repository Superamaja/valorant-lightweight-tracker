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

## Implemented (v1) — 2026-08-24

Built in `src/` against `docs/ipc-contract.md`. Structure: `ipc/` (types + the two commands
and one event), `hooks/useTracker.ts` (start -> initial snapshot -> event; a snapshot with an
older `lastUpdated` never overwrites a newer one), `lib/` (`table.ts` = the single column list
every row and the legend build their grid from, plus the ally/enemy tint tokens; `players.ts`
= team split, party colours, enrichment check; `format.ts`; `profile.ts` = tracker.gg link +
clipboard), `components/` (Header, StatusScreen, PlayerTable -> TeamBlock -> PlayerRow ->
`cells/`).

Decisions taken while building, beyond the baseline above:

- **Skin columns are Ingame-only.** Loadouts do not exist in pregame, so agent select drops
  both columns instead of showing two dead ones; the name column takes the space.
- **Default and random skins are shown as the words "Default" / "Random".** valorant-api has
  no artwork for them — every one answers with the same 14.5 KB "no image" placeholder (a box
  with an X), at both the skin and the level-1 icon, so there is no image to prefer.
- **Tooltips are native `title` attributes.** Zero JS, zero dependencies, and it is what
  "rank names are tooltips only" needs.
- **"Still loading" is inferred, not flagged.** The IPC contract has no field saying a
  snapshot is the fast one, and an absent heavy stat is identical to "player has no data", so
  the UI treats a snapshot where *no* row has any heavy stat as still loading and shows
  skeletons; after that an absent value renders "N/A". A lobby where nobody has ever played a
  competitive match would sit on skeletons — accepted, it is not reachable in practice.
- **Win rate never shows a skeleton** — it ships with the fast snapshot.
- **Column order** extends the baseline row for the phase-2 columns: agent · name · Vandal ·
  Phantom · rank+RR · peak · HS% · WR · last-5 pips · ΔRR · level (skins sit between name and
  rank; level moved to the end).
- **Level is hidden for incognito players** (the contract says it is withheld; the backend
  only withholds it for the separate "hide my level" flag, so the UI enforces it).
- **Colour budget**: coral accent + neutrals, ally blue / enemy red, and desaturated
  green/red for win-loss signals only (pips and ΔRR). Party dots use their own five-colour
  palette, distinct from all of the above.

Flagged against `docs/ipc-contract.md` (no guesses made, current behaviour noted above):

1. No way to tell a fast snapshot from an enriched one — inferred as described.
2. `accountLevel` for incognito players: the contract says withheld, `assemble.rs` withholds
   only on `hide_account_level`. UI hides it.
3. `agentSelectionState` is documented as `"locked" | "selected" | null` but is Riot's raw
   string; typed as `string` in the UI and compared against those two values.

## Revision 2 (agreed 2026-08-24, user feedback after first live run)

- **Level column removed.** Level renders as a small badge on the agent portrait (corner) instead. Level `null` OR `0` = hidden: show nothing (0 is Riot's "hidden" wire value; backend nulls it).
- **Peak rank cell un-dimmed**: same icon size and full color as the current-rank icon, plus the peak's episode/act short label next to it, formatted capitalized with colon+space: `E6: A3` / `V26: A1`. Backend exposes the label pre-formatted (new contract field).
- **HS% scope labeled**: header/tooltip says it covers the last 3 competitive matches (backend constant; changeable).
- **Rows slightly thicker** so the agent portrait can be larger and carry the level badge.
- Sizing must be verified against an emulated 10-player lobby at 1000x700 (no horizontal overflow, no scrolling), fixture removed after.

## Open items

- User may still provide reference images; revisit style section when they do.
- Score display in header: later, once backend exposes it.
- Not yet seen against a live match: party dot colours, the fast -> enriched hand-off, real
  skin art, and a full ten-row lobby in the real window.
