/**
 * Frontend mirror of the Rust `TrackerSnapshot` tree, 1:1 with `docs/ipc-contract.md`.
 * Nothing here is interpreted — the backend sends display-ready values.
 */

export type AppStatus = "ValorantNotRunning" | "Menus" | "Pregame" | "Ingame";

/**
 * Outcome of one recent competitive match. Real result when the match-details were read,
 * else the RR sign. "Draw" only comes from match-details; "Unknown" = 0-RR match without them.
 */
export type MatchResult = "Win" | "Loss" | "Draw" | "Unknown";

/**
 * Matches in a full HS%/KD window. `statsMatches` below this means a partial window. Mirrors
 * the backend's `RECENT_MATCHES_FOR_HS` (`src-tauri/src/riot/constants.rs`); keep the two in step.
 */
export const STATS_WINDOW = 5;

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
  /**
   * False for a mode's stand-in agent (Gauntlet: Glitched's draft KAY/O). Missing on
   * snapshots captured before the field existed: treat as playable.
   */
  playable?: boolean;
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

/**
 * Which of a row's stat groups are still in flight. A group is a skeleton while its flag is
 * true; once it is false an absent value is final ("N/A"), not loading.
 */
export interface PendingStats {
  name: boolean;
  /** Current rank, RR, leaderboard, peak, peak act and win rate — all one MMR payload. */
  rank: boolean;
  /** ΔRR and the last-5 pips. */
  history: boolean;
  /** HS% and KD. */
  recentStats: boolean;
  /** Vandal and Phantom skins. */
  skins: boolean;
}

export interface PlayerRow {
  /** puuid — a stable React key only. */
  id: string;
  name: string | null;
  incognito: boolean;
  /** Riot's internal team id. Never use it for colour — colour by `isAlly`. */
  team: string;
  isAlly: boolean;
  /**
   * Multi-team modes only (e.g. duos): the side this player is on, from 1. Null in two-team
   * modes; missing on snapshots captured before the field existed.
   */
  group?: number | null;
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

  /** Per-group loading state. Read it through `pendingOf` — old snapshots predate it. */
  pending: PendingStats;

  // Stats that arrive after the first paint — each cell is a skeleton until its group settles.
  winRate: WinRate | null;
  rrChange: number | null;
  recentResults: MatchResult[];
  headshotPercent: number | null;
  /** Kills/deaths over the same match window as `headshotPercent`, rounded to 2 decimals. */
  kd: number | null;
  /**
   * Matches HS% and KD were computed from (`STATS_WINDOW` when full), null when both are null.
   * Absent on snapshots captured before it existed.
   */
  statsMatches: number | null;
  vandalSkin: SkinInfo | null;
  phantomSkin: SkinInfo | null;
}

export interface TrackerSnapshot {
  status: AppStatus;
  map: MapInfo | null;
  mode: string | null;
  /** Server picker label, e.g. "US West (N. California)". Older captures lack it. */
  server: string | null;
  /** Upper-case region code ("NA", "EU", ...). Diagnostics only; not rendered. */
  region: string | null;
  ownTeam: string | null;
  /** Pre-ordered by the backend (ally block first, self first). Never re-sort. */
  players: PlayerRow[];
  /** Epoch milliseconds. */
  lastUpdated: number;
  /**
   * Every stat has settled. Guaranteed to imply no row carries a pending flag, so it is a
   * summary, not a gate: cells key their skeletons off `PlayerRow.pending`.
   */
  enriched: boolean;
  message: string | null;
}
