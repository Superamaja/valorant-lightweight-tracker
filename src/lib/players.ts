import type { PlayerRow } from "../ipc/types";

/**
 * Split into team blocks. `players` is already ordered by the backend (ally block first,
 * self first) — this preserves that order and must never sort.
 */
export function splitTeams(players: PlayerRow[]): { allies: PlayerRow[]; enemies: PlayerRow[] } {
  const allies: PlayerRow[] = [];
  const enemies: PlayerRow[] = [];
  for (const player of players) (player.isAlly ? allies : enemies).push(player);
  return { allies, enemies };
}

export interface PartyMark {
  color: string;
  size: number;
}

/** Dot colours for party grouping — distinct from the team, win and loss colours. */
const PARTY_COLORS = ["#e0b341", "#a78bfa", "#22d3ee", "#f472b6", "#a3e635"];

/** partyId -> dot colour + party size, assigned in row order. Solo players get no entry. */
export function partyGroups(players: PlayerRow[]): Map<string, PartyMark> {
  const sizes = new Map<string, number>();
  for (const { partyId } of players) {
    if (partyId) sizes.set(partyId, (sizes.get(partyId) ?? 0) + 1);
  }

  const marks = new Map<string, PartyMark>();
  for (const { partyId } of players) {
    if (!partyId || marks.has(partyId)) continue;
    const size = sizes.get(partyId) ?? 0;
    if (size < 2) continue;
    marks.set(partyId, { color: PARTY_COLORS[marks.size % PARTY_COLORS.length], size });
  }
  return marks;
}

/** Incognito players are shown by agent, never de-anonymized. */
export function displayName(player: PlayerRow): string {
  if (player.name) return player.name;
  if (player.incognito) return player.agent?.name || "Hidden player";
  return "Unknown player";
}
