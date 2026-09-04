# UI Spec (v1 baseline)

Last updated: 2026-08-24. Status: baseline agreed in conversation; no reference images yet — user may supply some later.

> **PROCESS GATE (revised 2026-08-24):** Claude may now spawn UI implementation subagents directly (the standard implementation model routing is acceptable to the user). The prompt still needs the same rigor as the old handoff prompts: full spec, file paths, constraints, verification steps. Codex review after the agent finishes still applies (the review rule switched from Fable to Codex, 2026-08-24 — see CLAUDE.md). (Historical: UI was previously user-run only — Claude wrote a ready-to-paste prompt and the user ran it with their preferred model at higher effort.)

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
- **Row, left→right:** agent portrait · name#tag · current rank icon (+RR) · peak rank icon (smaller, full colour) · account level. Party members marked with a matching colour bar. (Superseded in detail by the revisions below.)

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
- **Skeletons are per cell, driven by `PlayerRow.pending`.** Each cell shows a skeleton iff
  its group's flag is true, and renders "N/A" the moment the flag clears, so the table fills
  in a group at a time as the backend publishes its progress snapshots. Rank, peak, WR and the
  name can all skeleton now: they wait on the MMR and name payloads like every other stat, and
  a placeholder Unranked (or "Unknown player") before those land would be a wrong answer
  rather than a missing one. Read the flags through `pendingOf(player)` — snapshots captured
  before the field existed must read as settled, not as an all-skeleton table.
- **`TableLayout.loading` is gone**, and nothing keys off `snapshot.enriched`: it is a
  whole-snapshot summary, not a per-cell gate.
- **Column order** extends the baseline row for the phase-2 columns: agent · name · rank+RR ·
  peak · HS% · KD · WR · last-5 pips · ΔRR · Vandal · Phantom (revision 3 moved the skins to
  the end; level moved off the row entirely, see revision 2).
- **Level is hidden for incognito players** (the contract says it is withheld; the backend
  only withholds it for the separate "hide my level" flag, so the UI enforces it).
- **Colour budget**: coral accent + neutrals, ally blue / enemy red, and desaturated
  green/red for win-loss signals only (pips and ΔRR). Party dots use their own five-colour
  palette, distinct from all of the above.

Flagged against `docs/ipc-contract.md` (no guesses made, current behaviour noted above):

1. ~~No way to tell a fast snapshot from an enriched one — inferred as described.~~ Resolved:
   `PlayerRow.pending` carries per-group loading state (2026-08-25).
2. `accountLevel` for incognito players: the contract says withheld, `assemble.rs` withholds
   only on `hide_account_level`. UI hides it.
3. `agentSelectionState` is documented as `"locked" | "selected" | null` but is Riot's raw
   string; typed as `string` in the UI and compared against those two values.

## Revision 2 (agreed 2026-08-24, user feedback after first live run)

- **Level column removed.** Level renders as a small badge on the agent portrait (corner) instead. Level `null` OR `0` = hidden: show nothing (0 is Riot's "hidden" wire value; backend nulls it).
- **Peak rank cell un-dimmed**: same icon size and full color as the current-rank icon, plus the peak's episode/act short label next to it, formatted capitalized with colon+space: `E6: A3` / `V26: A1`. Backend exposes the label pre-formatted (new contract field).
- **HS% scope labeled**: header/tooltip says it covers the last 5 competitive matches (backend constant; changeable).
- **Rows slightly thicker** so the agent portrait can be larger and carry the level badge.
- Sizing must be verified against an emulated 10-player lobby at 1000x700 (no horizontal overflow, no scrolling), fixture removed after.

## Revision 3 (polish pass, 2026-08-25)

- **Column order**: identity → rank → stats → skins. Vandal and Phantom now sit after ΔRR at
  the right edge; skin art keeps its opacity. Vandal carries the wider floor of the two, since
  it is the column that meets the stat cluster.
- **Row plate**: the team tint fades to a faint constant instead of to transparent, so a row
  keeps its shape across the stat columns. `lib/table.ts` holds the A/B: `ROW_PLATE` points at
  `ROW_PLATE_ON` (`to-white/[0.015]`) or `ROW_PLATE_OFF` (`to-transparent`) — one line.
- **Rank sizes**: current rank icon `h-8`, peak `h-6`, peak still full colour. The agent
  portrait is unchanged.
- **Unranked** shows the rank icon alone; the em-dash that stood in for RR is gone, and the
  rank name stays as the tooltip.
- **`YOU` badge is amber**, not the coral accent, so it does not read as another accent
  element. Enemy red is untouched.
- **Party marker is a thin vertical bar** (`w-0.5 h-6`) rather than a dot, same palette colour
  and same show-only-when-in-a-party rule.
- **Snapshot age is hidden while live.** The header's "Xm ago" appears only on the held
  last-match table or once a live snapshot passes 90s without a refresh — a healthy live match
  refreshes itself, so the age is noise.

## Last match: waiting screen first (2026-08-26)

- Leaving a match no longer leaves its table up. On `Menus` the app shows the normal waiting
  screen, and the held snapshot sits behind a "View last match" button under the subtitle —
  visible, but not the thing you land on.
- Pressing it swaps in the held table unchanged: the "Last match" chip, the always-shown
  snapshot age, all of it. A quiet "← Back" next to the chip returns to the waiting screen.
- The toggle is a plain `useState` in `App` and resets on any non-`Menus` status, so entering
  agent select or a match always shows the live table, and the button always opens the newest
  finished match.
- How the snapshot is held is unchanged: still the single `seen` ref written after commit, and
  still only plain `Menus` offers it.

## Version / updater affordance (2026-08-25, reworked 2026-08-26)

- **The version lives on the status screen, not the header.** A quiet `v{version}` line sits
  under the subtitle of every status screen, same weight as the "last updated" text. The
  version comes from `package.json` through Vite's `define` (`__APP_VERSION__`), so it costs
  nothing at runtime and works in plain-browser dev too.
- That line is also the manual check control: clicking it re-runs the check, pulsing while it
  is in flight, then showing "Up to date" or "Check failed" for a few seconds before falling
  back to the version. No popup, no dialog. (One automatic check also runs at app start, from
  the always-mounted header.)
- **An available update owns exactly one affordance per screen** (2026-08-26): on any status
  screen it is a prominent solid-accent CTA button under the subtitle — `Update to v{version}`,
  disabled + pulsing as `Updating` while installing, `Retry update to v{version}` after a
  failed install (error text in the tooltip). The version line then just shows the plain
  version, no accent flash. Over a match table (live or held) the affordance is instead the
  small header accent chip `Update: v{version}` — `App` passes `showUpdate` (does the main
  area show a table?) to `Header`, so chip and CTA never render together. Up to date, failed
  and never-checked render nothing in the header — it stays about the match.
- Check and install state is shared so every surface agrees: a module-level store in
  `src/lib/updater.ts` (`runUpdateCheck` / `runUpdateInstall` / `subscribeUpdateState`), read
  through `src/hooks/useUpdateState.ts`. No state library. Both surfaces call the real
  backend commands (`check_update` / `apply_update`, see `docs/ipc-contract.md`); clicking
  install downloads, swaps and restarts the app.

## Diagnostics line (2026-09-03)

- **Every status screen ends in one quiet 10px row**: `v{version} · Copy diagnostics`, the two
  separated by a middle dot. The version keeps its job as the manual update check; the second
  item copies the backend's plain-text report (`get_diagnostics`, see `docs/ipc-contract.md`)
  for pasting into a GitHub issue. Same weight as the "last updated" text, so the row reads as
  a footer and not as a second call to action.
- **States** are the same on both surfaces: `Copy diagnostics` idle, `Copying` (pulsing,
  disabled) while the report is being built, `Copied` in the win green for two seconds, then
  back to idle. A refused clipboard gives `Copy failed` and stays there until a later copy
  works.
- **Fallback**: on `Copy failed` the status screen prints the report under the row in a
  read-only, pre-selected, `select-text` monospace textarea with a `Select all and copy` hint,
  because the users who need the report are the ones whose setup is broken. It is a fixed
  384px wide, grows with the report to a 160px cap, and sits out of flow below the row so the
  centred title/ring/subtitle stack does not move when it appears.
- **Over a table the header carries it instead**, quietly: the same item, same weight, left of
  the update chip so the chip keeps the priority. No textarea there. It appears only while rows
  are on screen (live Pregame/Ingame, or the held last-match table) and renders nothing
  otherwise, so an ordinary match header is unchanged.
- The report needs to know what the user is looking at, which is the one thing the backend
  cannot see: the frontend sends the visible screen title, or `Player table` / `Last match
  table` from the header, plus whether a held table is in play. `App` already knows both.
- Behaviour lives in `src/hooks/useCopyDiagnostics.ts` so the two surfaces cannot drift; the
  clipboard call is the existing `copyText` from `src/lib/profile.ts`, no new dependency. With
  no backend (plain-browser dev) the copy is a short "diagnostics unavailable" stub rather than
  an error.

## Open items

- User may still provide reference images; revisit style section when they do.
- Score display in header: later, once backend exposes it.
- Not yet seen against a live match: party dot colours, the incremental fill-in, real skin
  art, and a full ten-row lobby in the real window.
