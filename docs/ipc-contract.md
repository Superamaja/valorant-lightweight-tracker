# IPC Contract (Rust backend ↔ React frontend)

Last updated: 2026-08-25 (phase 2 stats added; `kd` added). Status: backend implemented; this is the
exact TypeScript-facing contract the UI agent builds against. Everything the frontend
receives is already display-ready — no Riot-API-shape interpretation happens in the UI.

The backend exposes **two Tauri commands** and emits **one Tauri event**. All payloads are
`serde`-serialized with `camelCase` field names.

---

## Commands

Invoke via `@tauri-apps/api/core`'s `invoke`.

### `start_tracker`

```ts
import { invoke } from "@tauri-apps/api/core";
await invoke("start_tracker"); // returns void
```

Starts the background loop (connect when Valorant appears, reconnect on loss, emit on every
change). **Idempotent** — call it once on app mount; repeat calls are no-ops. Call this
before (or right after) subscribing to the event.

### `get_tracker_state`

```ts
const snapshot = await invoke<TrackerSnapshot>("get_tracker_state");
```

Returns the current `TrackerSnapshot` on demand (same shape the event carries). Use it once
on mount to render immediately without waiting for the next event.

---

## Event

Name: **`tracker-state`**. Payload: a `TrackerSnapshot` (below). Emitted on every resolved
state change (dedup'd — identical snapshots are not re-emitted).

**Two-phase snapshots at match start.** Entering Pregame or Ingame fires **two** events in
quick succession for the same match (the heavy per-player stats take several seconds of
sequential fetching, so the backend does not make the UI wait for them):

1. **Fast snapshot** — as soon as names + MMR are in: `status`, `map`, `mode`, `ownTeam`,
   and per row the `name`, `agent`, `currentRank`, `rr`, `leaderboardRank`, `peakRank`,
   `peakRankAct`, `accountLevel`, `partyId`, and `winRate` (WR is derived from the MMR payload, so it costs
   no extra request). The heavier fields are **not yet populated**: `rrChange` is `null`,
   `recentResults` is `[]`, `headshotPercent` and `kd` are `null`, and `vandalSkin` /
   `phantomSkin` are `null`.
2. **Enriched snapshot** — a moment later, the same rows with `rrChange`, `recentResults`,
   `headshotPercent`, `kd`, and (Ingame only) `vandalSkin` / `phantomSkin` filled in.

The UI **must render the fast snapshot immediately** and let those fields fill in on the
enriched event. **Key loading skeletons on the snapshot's `enriched` flag: `enriched === false`
is the fast phase-1 snapshot (show skeletons for the heavy fields); `enriched === true` means
the heavy fields are final.** This supersedes the previous inference of "still loading" from
data absence (treating a snapshot where no row had any heavy stat as the fast one) — that
heuristic is no longer needed and must not be used. Once `enriched === true`, a `null`/empty
heavy field means the player genuinely has no data for it (render the "N/A" placeholder), not
"still loading". One bounded exception: if a player's stat fetches keep failing transiently, the
backend retries the failed players a few times (publishing interim `enriched: false` snapshots),
then gives up for that lobby and publishes `enriched: true` with the residual nulls — so a null
under `enriched: true` means "no data obtainable this match", which the UI renders as N/A either
way. Re-entering an already-loaded match
(the score changes each round) emits a **single** already-enriched snapshot. A Pregame→Ingame
transition re-runs the two phases for the now-visible enemy team. All of this rides the
existing dedup, so listeners need no special handling beyond expecting stats to arrive on a
later event than the row itself.

```ts
import { listen } from "@tauri-apps/api/event";

const unlisten = await listen<TrackerSnapshot>("tracker-state", (event) => {
  const snapshot = event.payload;
  // re-render
});
// later: unlisten();
```

Recommended startup sequence:

```ts
await invoke("start_tracker");
const initial = await invoke<TrackerSnapshot>("get_tracker_state");
render(initial);
const unlisten = await listen<TrackerSnapshot>("tracker-state", (e) => render(e.payload));
```

---

## Types

```ts
type AppStatus =
  | "ValorantNotRunning" // lockfile missing, client not running, or connection lost
  | "Menus"              // in menus, no active match
  | "Pregame"            // agent select (only your own team is visible)
  | "Ingame";            // live match (full roster)

interface TrackerSnapshot {
  status: AppStatus;
  map: MapInfo | null;          // resolved map; null in Menus / ValorantNotRunning
  mode: string | null;          // display mode name (see below); null outside a match
  ownTeam: string | null;       // local player's team id ("Red"|"Blue"); null outside a match
  players: PlayerRow[];         // [] in Menus / ValorantNotRunning
  enriched: boolean;            // false ONLY on the fast phase-1 snapshot (heavy stats still
                                // loading); true on the enriched snapshot, on re-entry to an
                                // already-loaded match, and on all non-match states
  lastUpdated: number;          // epoch milliseconds this snapshot was produced
  message: string | null;       // optional status line, e.g. "Waiting for Valorant..."
}

interface MapInfo {
  id: string;                   // raw MapID path, e.g. "/Game/Maps/Ascent/Ascent"
  name: string;                 // "Ascent" ("" if unresolved from static data)
  splashUrl: string | null;     // valorant-api splash image URL
  listViewUrl: string | null;   // valorant-api list-view icon URL
}

interface PlayerRow {
  id: string;                   // puuid — a stable, opaque React key only
  name: string | null;          // "GameName#TagLine", or null when hidden (incognito) / unresolved
  incognito: boolean;           // player enabled streamer/incognito mode — do NOT de-anonymize
  team: string;                 // raw team id ("Red"|"Blue")
  isAlly: boolean;              // on the local player's team
  isSelf: boolean;              // this row is the local player
  agent: AgentInfo | null;      // null when not yet selected (pregame) / unresolved
  agentSelectionState: string | null; // pregame only; Riot's raw CharacterSelectionState
                                       // passed through. Known values "locked" | "selected";
                                       // other strings possible. null outside pregame
  currentRank: RankInfo;        // current competitive rank
  rr: number;                   // ranked rating 0–100 (0 when unranked)
  leaderboardRank: number;      // leaderboard position; nonzero only for Ascendant+ top players
  peakRank: RankInfo;           // highest rank across all recorded seasons
  peakRankAct: string | null;   // act the peak was set in: "E6: A3", "V26: A1"; null when no
                                // season could be attributed to the peak
  accountLevel: number | null;  // null when hidden from this viewer (see rules below)
  partyId: string | null;       // grouping id; set only when the player is in a party of >1

  // --- Phase 2 per-player stats ---
  winRate: WinRate | null;      // current-season WR; null when 0 games this season
  rrChange: number | null;      // ΔRR of the player's newest competitive match; null if none
  recentResults: MatchResult[]; // up to 5 recent comp results, newest first; [] when none
  headshotPercent: number | null; // HS% over recent comp matches (0–100); null when "N/a"
  kd: number | null;            // kills/deaths over the same recent comp matches, 2 decimals;
                                // null when those matches carry no stats for the player
  vandalSkin: SkinInfo | null;  // equipped Vandal skin; null in Pregame / Menus
  phantomSkin: SkinInfo | null; // equipped Phantom skin; null in Pregame / Menus
}

interface WinRate {
  percent: number;              // wins/games*100, rounded int (the "57" in vRY "57 (14)")
  games: number;                // competitive games this season (the "(14)")
}

type MatchResult = "Win" | "Loss" | "Unknown"; // "Unknown" = 0-RR match (ambiguous sign)

interface SkinInfo {
  name: string;                 // "Neptune Vandal" ("" if unresolved)
  iconUrl: string | null;       // valorant-api displayIcon URL
}

interface AgentInfo {
  id: string;                   // agent uuid (lowercased)
  name: string;                 // "Jett" ("" if unresolved)
  iconUrl: string | null;       // valorant-api displayIcon URL
}

interface RankInfo {
  tier: number;                 // CompetitiveTier number (0 = Unranked)
  name: string;                 // "Immortal 2", "Unranked", ...
  iconUrl: string | null;       // competitivetiers large/small icon URL; null for Unranked
}
```

---

## Field semantics & guarantees

- **Status drives layout.** `ValorantNotRunning` and `Menus` carry an empty `players` array.
  `ValorantNotRunning` renders the waiting state. For `Menus` the shipped UI intentionally
  **holds the last match**: if a match table was rendered earlier this app session, plain
  `Menus` keeps that table on screen with a "Last match" chip in the header instead of the
  empty state (the empty waiting screen shows only when no match has been seen yet). This is
  a frontend-only display choice — the `Menus` snapshot itself still carries no players.
  `Pregame` carries **only the local player's team** (max 5
  rows) — Riot does not expose enemies during agent select; this is a platform limit, not a
  bug. `Ingame` carries the full roster.
- **Row order is guaranteed by the backend — the UI must NOT re-sort.** `players[]` is always
  ordered: (1) the local player's team (`isAlly === true`) first, then the enemy team;
  (2) within the ally block the local player (`isSelf === true`) is first; (3) the remaining
  rows in each block are sorted by display name (case-insensitive), with hidden/unresolved
  names (`name === null`, e.g. incognito) last, tie-broken by `id` (puuid) for stability.
  Render rows in array order. **Colour by `isAlly` only** (ally block = blue, enemy = red);
  do not read the raw `team` id for colour — it is Riot's internal Red/Blue label, exposed
  only for debugging, and does not correspond to the ally/enemy split.
- **`mode`** is one of: `Competitive`, `Unrated`, `Swiftplay`, `Spike Rush`, `Deathmatch`,
  `Escalation`, `Replication`, `Team Deathmatch`, `Custom`, `Snowball Fight`,
  `All Random One Site`, `Knockout`, `New Map`, or `Custom Game` (customs). Unknown queue ids
  pass through as their raw string.
- **`name` = null** when `incognito` is true (except for your own row) or the name-service did
  not return a name. Always guard for null and render a placeholder.
- **`accountLevel` = null** when the player is incognito, OR set "hide my level" — in either
  case only when they are neither you nor a member of your party. You and your party members
  always see the real level (the backend never nulls the self row). **Wire caveat:** Riot
  itself zeroes `AccountLevel` in the coregame `PlayerIdentity` when the hide-level flag is set,
  even for your own row, so an in-match self level may read `0` off the wire; your MENUS
  presence carries the real level. The backend does not backfill from presence — it treats a
  wire `0` as "hidden by Riot" and emits `null`, self row included. **`accountLevel` is
  therefore never `0`:** the only values are a real level or `null`.
- **`agent` = null / `agentSelectionState`** — `agentSelectionState` is Riot's raw
  `CharacterSelectionState` string passed straight through (**not** a closed enum): type it as
  `string | null`. `"locked"` and `"selected"` are the known values, but treat any other string
  as possible. During Pregame a teammate who hasn't locked yet has `agent: null` and
  `agentSelectionState: "selected"` (still picking) or `null`; once locked, `agent` is populated
  and `agentSelectionState: "locked"`. It is `null` outside Pregame.
- **Ranks never error a row.** A private profile / new account / API hiccup yields
  `tier: 0, name: "Unranked", iconUrl: null` and `rr: 0` for that player only.
- **`peakRank`** already accounts for the pre-Ascendant tier renumbering — treat `tier` as a
  modern tier number and map it with the same icons as `currentRank`.
- **`peakRankAct`** is a display-ready short label for the act that peak was set in:
  `"E{episode}: A{act}"` (`"E6: A3"`), or `"{episode}: A{act}"` when the episode identifier
  already carries a letter — the V-era naming (`"V26: A1"`). Which episode/act an id maps to
  is derived exactly as vRY does; only the formatting differs (vRY prints `(e6a3)`). It is
  `null` when the peak is the player's current tier with no better season on record (nothing
  to attribute it to) or the season id is missing from the content service. Render it
  verbatim next to the peak icon.
- **Party grouping** — players sharing a `partyId` are in the same party. `null` means solo
  (or a party of one). Use it to draw the matching-dot / bracket accent from the UI spec.
- **`iconUrl` / `splashUrl` etc.** are direct valorant-api.com PNG URLs. They are stable per
  game patch. The backend caches the static-data *mappings*; the images themselves are plain
  URLs the UI loads (and may cache) directly.
- **`lastUpdated`** is epoch ms — use it for the "last updated" text in the header.

### Phase 2 stat fields

- **All phase-2 stats are populated for every player, incognito included** — a hidden player
  still shows WR / ΔRR / last-5 / HS% / KD / skins; only `name` and `accountLevel` are withheld
  (their puuid is known, so the stats are real). This matches vRY.
- **`winRate`** is derived from the same MMR payload used for ranks (no extra request):
  `{ percent, games }` where `percent = round(NumberOfWins / NumberOfGames * 100)`. `null`
  when the player has 0 competitive games this season (render "N/A"). vRY renders this as
  `"{percent} ({games})"`, e.g. `"57 (14)"`.
- **`rrChange`** is the newest competitive match's `RankedRatingEarned` (can be negative,
  zero, or positive). `null` when the player has no recent competitive matches.
- **`recentResults`** is up to 5 recent competitive results, **newest first**, each derived
  from `RankedRatingEarned` sign: `> 0 → "Win"`, `< 0 → "Loss"`, `= 0 → "Unknown"` (a 0-RR
  match is genuinely ambiguous — draw / placement / AFK-adjusted; render it as a neutral pip,
  not a loss). `[]` when the player has no recent competitive matches.
- **`headshotPercent`** is an integer 0–100 computed over the last few competitive matches
  (`headshots / (headshots + bodyshots + legshots)`, matching vRY). `null` when the player
  has no recent matches (render "N/a").
- **`kd`** is total kills / total deaths across the **same** recent competitive matches HS% is
  computed from — it rides those already-downloaded match-details payloads and costs **zero
  extra requests**. Rounded to 2 decimals (e.g. `1.28`); a window with 0 deaths yields the kill
  count itself (7 kills, 0 deaths -> `7`). `null` when the player has no recent competitive
  matches, or when the fetched matches carry no per-player stats entry for that player. Render
  "N/A" for null. The two fields ride the same payload and are cached/withheld together, but
  their null cases are **not identical**: `headshotPercent` is also `null` when the window
  records zero shots, so a row can legitimately show `kd` with an "N/a" HS% — treat each
  field's null independently.
- **`vandalSkin` / `phantomSkin`** are populated only in `Ingame` (Riot exposes loadouts only
  once the match starts) — both are `null` in `Pregame` and `Menus`. A `SkinInfo` with an
  empty `name` means the skin uuid was not in the static-data cache (render the icon if
  present, otherwise a placeholder).

## Notes for the UI

- There is no manual refresh — the event fires on every change. Do not poll
  `get_tracker_state` on a timer; use it only for the initial paint.
- Copy-to-clipboard (UI spec: click a row → copy `name#tag`) should use `row.name` and no-op
  when it is null.
- The backend never throws for "game not running" — that arrives as a normal
  `status: "ValorantNotRunning"` snapshot with a friendly `message`.
