import type { PlayerRow as Player } from "../ipc/types";
import { TEAM_TINT, type TableLayout, type TeamSide } from "../lib/table";
import { PlayerRow } from "./PlayerRow";

/** One team: a hairline heading plus its rows, in the order the backend sent them. */
export function TeamBlock({
  side,
  label,
  players,
  layout,
}: {
  side: TeamSide;
  label: string;
  players: Player[];
  layout: TableLayout;
}) {
  const tint = TEAM_TINT[side];

  return (
    <section className="mt-4 first:mt-2">
      <div className="mb-1.5 flex items-center gap-2 pl-2">
        <span className={`h-1.5 w-1.5 rounded-full ${tint.dot}`} />
        <h2 className="text-[10px] font-medium tracking-[0.18em] text-neutral-400 uppercase">
          {label}
        </h2>
        <span className="text-[10px] tabular-nums text-neutral-600">{players.length}</span>
        <span className={`h-px flex-1 bg-linear-to-r ${tint.rule} to-transparent`} />
      </div>

      <div className="flex flex-col gap-1">
        {players.map((player) => (
          <PlayerRow key={player.id} player={player} side={side} layout={layout} />
        ))}
      </div>
    </section>
  );
}
