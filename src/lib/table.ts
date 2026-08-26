import type { PartyMark } from "./players";

/**
 * One source of truth for the table's columns: the legend and every row build their CSS
 * grid from this list, in this order.
 */
export interface Column {
  key: string;
  /** Legend text. Empty for the icon-only columns. */
  label: string;
  /** CSS grid track. */
  width: string;
  /** Cell content starts at the left edge, so the legend must too. */
  left?: boolean;
  /** Legend tooltip, for a label too short to carry its own scope. */
  hint?: string;
}

/**
 * How many recent competitive matches HS% is computed over. Mirrors the backend's
 * `RECENT_MATCHES_FOR_HS` (`src-tauri/src/riot/constants.rs`) — keep the two in step.
 */
export const HS_MATCH_WINDOW = 5;
export const HS_SCOPE = `last ${HS_MATCH_WINDOW} competitive matches`;

/**
 * Every track except the two icon columns is `minmax(floor, Nfr)`, so spare width spreads
 * across the whole row instead of pooling into one column. The floors are the widths the
 * table needs to stay readable, and their sum plus the gaps is what fits a 1000px window.
 * Player takes the largest share; the stat cluster on the right takes one each so it
 * breathes rather than staying pinned at its floor.
 */
const COLUMNS: Column[] = [
  { key: "party", label: "", width: "8px" },
  { key: "agent", label: "", width: "44px" },
  { key: "name", label: "Player", width: "minmax(120px,3fr)", left: true },
  // Skin art is ~4.3:1, so in a 24px-tall cell its width always wins and it fills whatever
  // track it is given. `SKIN_ART_WIDTH` caps it instead, and the leftover becomes the margin
  // that keeps the guns off the name and off the rank icon. Phantom's floor is the wider of
  // the two because it is the one that butts up against the rank column.
  { key: "vandal", label: "Vandal", width: "minmax(96px,1fr)" },
  { key: "phantom", label: "Phantom", width: "minmax(108px,1fr)" },
  { key: "rank", label: "Rank", width: "minmax(92px,1fr)", left: true },
  { key: "peak", label: "Peak", width: "minmax(78px,1fr)", left: true },
  { key: "hs", label: "HS%", width: "minmax(46px,1fr)", hint: `Headshot % over the ${HS_SCOPE}` },
  { key: "kd", label: "KD", width: "minmax(46px,1fr)", hint: `Kills/deaths over the ${HS_SCOPE}` },
  { key: "wr", label: "WR", width: "minmax(62px,1fr)" },
  { key: "recent", label: "Last 5", width: "minmax(56px,1fr)" },
  { key: "delta", label: "ΔRR", width: "minmax(46px,1fr)" },
];

/**
 * How wide the skin artwork may draw. Well under the skin tracks' floors on purpose: the
 * remainder is centred slack, so the gun keeps clear air on both sides instead of running
 * edge to edge and leaving only the grid gap between it and the next column.
 */
export const SKIN_ART_WIDTH = "max-w-22";

/** Loadouts only exist once a match starts, so the skin columns are Ingame-only. */
const SKIN_KEYS = new Set(["vandal", "phantom"]);

export const visibleColumns = (withSkins: boolean): Column[] =>
  withSkins ? COLUMNS : COLUMNS.filter((column) => !SKIN_KEYS.has(column.key));

export const gridTemplate = (withSkins: boolean): string =>
  visibleColumns(withSkins)
    .map((column) => column.width)
    .join(" ");

/** Shared by the legend and every row so the columns line up exactly. */
export const ROW_GRID = "grid items-center gap-x-2.5 border-l-2 pr-3 pl-2";

/** Everything a row needs that is the same for the whole table. */
export interface TableLayout {
  /** `grid-template-columns` value from `gridTemplate`. */
  template: string;
  withSkins: boolean;
  parties: Map<string, PartyMark>;
}

export type TeamSide = "ally" | "enemy";

/**
 * Team tints. Ally is always blue and enemy always red, keyed off `isAlly` — never off the
 * raw Riot team id.
 */
export const TEAM_TINT: Record<TeamSide, { dot: string; rule: string; row: string }> = {
  ally: {
    dot: "bg-ally",
    rule: "from-ally/30",
    row: "border-l-ally/50 from-ally/[0.07] hover:from-ally/[0.14]",
  },
  enemy: {
    dot: "bg-enemy",
    rule: "from-enemy/30",
    row: "border-l-enemy/50 from-enemy/[0.07] hover:from-enemy/[0.14]",
  },
};
