# IPC Contract (Rust backend ↔ React frontend)

Last updated: 2026-08-24. Status: backend implemented; this is the exact TypeScript-facing
contract the UI agent builds against. Everything the frontend receives is already
display-ready — no Riot-API-shape interpretation happens in the UI.

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
  agentSelectionState: string | null; // pregame only: "locked" | "selected" | null
  currentRank: RankInfo;        // current competitive rank
  rr: number;                   // ranked rating 0–100 (0 when unranked)
  leaderboardRank: number;      // leaderboard position; nonzero only for Ascendant+ top players
  peakRank: RankInfo;           // highest rank across all recorded seasons
  accountLevel: number | null;  // null when hidden from this viewer (see rules below)
  partyId: string | null;       // grouping id; set only when the player is in a party of >1
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

- **Status drives layout.** `ValorantNotRunning` and `Menus` carry an empty `players` array;
  render the waiting/empty state. `Pregame` carries **only the local player's team** (max 5
  rows) — Riot does not expose enemies during agent select; this is a platform limit, not a
  bug. `Ingame` carries the full roster.
- **`mode`** is one of: `Competitive`, `Unrated`, `Swiftplay`, `Spike Rush`, `Deathmatch`,
  `Escalation`, `Replication`, `Team Deathmatch`, `Custom`, `Snowball Fight`,
  `All Random One Site`, `Knockout`, `New Map`, or `Custom Game` (customs). Unknown queue ids
  pass through as their raw string.
- **`name` = null** when `incognito` is true (except for your own row) or the name-service did
  not return a name. Always guard for null and render a placeholder.
- **`accountLevel` = null** when the player set "hide my level" AND they are neither you nor a
  member of your party. You and your party members always see the real level.
- **`agent` = null / `agentSelectionState`** — during Pregame a teammate who hasn't locked yet
  has `agent: null` and `agentSelectionState: "selected"` (still picking) or `null`. Once
  locked, `agent` is populated and `agentSelectionState: "locked"`.
- **Ranks never error a row.** A private profile / new account / API hiccup yields
  `tier: 0, name: "Unranked", iconUrl: null` and `rr: 0` for that player only.
- **`peakRank`** already accounts for the pre-Ascendant tier renumbering — treat `tier` as a
  modern tier number and map it with the same icons as `currentRank`.
- **Party grouping** — players sharing a `partyId` are in the same party. `null` means solo
  (or a party of one). Use it to draw the matching-dot / bracket accent from the UI spec.
- **`iconUrl` / `splashUrl` etc.** are direct valorant-api.com PNG URLs. They are stable per
  game patch. The backend caches the static-data *mappings*; the images themselves are plain
  URLs the UI loads (and may cache) directly.
- **`lastUpdated`** is epoch ms — use it for the "last updated" text in the header.

## Notes for the UI

- There is no manual refresh — the event fires on every change. Do not poll
  `get_tracker_state` on a timer; use it only for the initial paint.
- Copy-to-clipboard (UI spec: click a row → copy `name#tag`) should use `row.name` and no-op
  when it is null.
- The backend never throws for "game not running" — that arrives as a normal
  `status: "ValorantNotRunning"` snapshot with a friendly `message`.
