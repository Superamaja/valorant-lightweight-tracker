/**
 * Frontend mirror of the Rust `TrackerSnapshot` tree, 1:1 with `docs/ipc-contract.md`.
 * Nothing here is interpreted — the backend sends display-ready values.
 */

export type AppStatus = "ValorantNotRunning" | "Menus" | "Pregame" | "Ingame";

/** Outcome of one recent competitive match. "Unknown" = 0-RR match (ambiguous sign). */
export type MatchResult = "Win" | "Loss" | "Unknown";

/** Pregame agent-pick state. Riot's raw string, known values "locked" | "selected". */
export const LOCKED = "locked";
export const SELECTED = "selected";

export interface MapInfo {
  id: string;
  name: string;
  splashUrl: string | null;
  listViewUrl: string | null;
}

export interface AgentInfo {
  id: string;
  name: string;
  iconUrl: string | null;
}

export interface RankInfo {
  /** CompetitiveTier number, 0 = Unranked. */
  tier: number;
  name: string;
  iconUrl: string | null;
}

export interface SkinInfo {
  name: string;
  iconUrl: string | null;
}

export interface WinRate {
  percent: number;
  games: number;
}

export interface PlayerRow {
  /** puuid — a stable React key only. */
  id: string;
  name: string | null;
  incognito: boolean;
  /** Riot's internal team id. Never use it for colour — colour by `isAlly`. */
  team: string;
  isAlly: boolean;
  isSelf: boolean;
  agent: AgentInfo | null;
  agentSelectionState: string | null;
  currentRank: RankInfo;
  rr: number;
  /** Nonzero only for Ascendant+ leaderboard players. */
  leaderboardRank: number;
  peakRank: RankInfo;
  /** Act the peak was reached in, e.g. "E6: A3" / "V26: A1". Null when unattributed. */
  peakRankAct: string | null;
  accountLevel: number | null;
  partyId: string | null;

  // Phase-2 stats. Absent on a match's first ("fast") snapshot — see `TrackerSnapshot.enriched`.
  winRate: WinRate | null;
  rrChange: number | null;
  recentResults: MatchResult[];
  headshotPercent: number | null;
  vandalSkin: SkinInfo | null;
  phantomSkin: SkinInfo | null;
}

export interface TrackerSnapshot {
  status: AppStatus;
  map: MapInfo | null;
  mode: string | null;
  ownTeam: string | null;
  /** Pre-ordered by the backend (ally block first, self first). Never re-sort. */
  players: PlayerRow[];
  /** Epoch milliseconds. */
  lastUpdated: number;
  /** False only on a match's fast snapshot while heavy stats are in flight — key skeletons on this. */
  enriched: boolean;
  message: string | null;
}
