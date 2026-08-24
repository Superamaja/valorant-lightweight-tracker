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
}

const COLUMNS: Column[] = [
  { key: "party", label: "", width: "8px" },
  { key: "agent", label: "", width: "40px" },
  { key: "name", label: "Player", width: "minmax(120px,1fr)", left: true },
  { key: "vandal", label: "Vandal", width: "104px" },
  { key: "phantom", label: "Phantom", width: "104px" },
  { key: "rank", label: "Rank", width: "92px", left: true },
  { key: "peak", label: "Peak", width: "34px" },
  { key: "hs", label: "HS%", width: "46px" },
  { key: "wr", label: "WR", width: "62px" },
  { key: "recent", label: "Last 5", width: "68px" },
  { key: "delta", label: "ΔRR", width: "46px" },
  { key: "level", label: "Lvl", width: "40px" },
];

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
  /** The enriched snapshot has not arrived yet — heavy stat cells show skeletons. */
  loading: boolean;
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
