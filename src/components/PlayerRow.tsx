import type { PlayerRow as Player } from "../ipc/types";
import { pendingOf, type PartyMark } from "../lib/players";
import { ROW_GRID, TEAM_TINT, type TableLayout, type TeamSide } from "../lib/table";
import { AgentCell } from "./cells/AgentCell";
import { NameCell } from "./cells/NameCell";
import { PeakCell, RankCell } from "./cells/RankCell";
import { SkinCell } from "./cells/SkinCell";
import { HeadshotCell, KdCell, ResultPips, RrChangeCell, WinRateCell } from "./cells/StatCells";

/** Same colour for everyone in a party. Solo players keep the column empty. */
function PartyDot({ mark }: { mark: PartyMark | undefined }) {
  if (!mark) return <span />;
  return (
    <span
      className="mx-auto block h-1.5 w-1.5 rounded-full"
      style={{ backgroundColor: mark.color }}
      title={`Party of ${mark.size}`}
    />
  );
}

/** One player. Cells are rendered in the column order defined by `lib/table.ts`. */
export function PlayerRow({
  player,
  side,
  layout,
}: {
  player: Player;
  side: TeamSide;
  layout: TableLayout;
}) {
  const tint = TEAM_TINT[side];
  const pending = pendingOf(player);

  return (
    <div
      style={{ gridTemplateColumns: layout.template }}
      className={`group h-12 rounded-r-md bg-linear-to-r to-transparent transition-colors ${ROW_GRID} ${tint.row} ${
        player.isSelf ? "bg-white/[0.04] ring-1 ring-white/10 ring-inset" : ""
      }`}
    >
      <PartyDot mark={player.partyId ? layout.parties.get(player.partyId) : undefined} />
      <AgentCell
        agent={player.agent}
        selectionState={player.agentSelectionState}
        level={player.accountLevel}
      />
      <NameCell player={player} />
      {layout.withSkins && (
        <SkinCell skin={player.vandalSkin} weapon="Vandal" pending={pending.skins} />
      )}
      {layout.withSkins && (
        <SkinCell skin={player.phantomSkin} weapon="Phantom" pending={pending.skins} />
      )}
      <RankCell
        rank={player.currentRank}
        rr={player.rr}
        leaderboardRank={player.leaderboardRank}
        pending={pending.rank}
      />
      <PeakCell rank={player.peakRank} act={player.peakRankAct} pending={pending.rank} />
      <HeadshotCell percent={player.headshotPercent} pending={pending.recentStats} />
      <KdCell kd={player.kd} pending={pending.recentStats} />
      <WinRateCell winRate={player.winRate} pending={pending.rank} />
      <ResultPips results={player.recentResults} pending={pending.history} />
      <RrChangeCell change={player.rrChange} pending={pending.history} />
    </div>
  );
}
