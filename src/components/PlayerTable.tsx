import type { TrackerSnapshot } from "../ipc/types";
import { partyGroups, splitTeams } from "../lib/players";
import { gridTemplate, ROW_GRID, visibleColumns, type TableLayout } from "../lib/table";
import { TeamBlock } from "./TeamBlock";

function Legend({ layout }: { layout: TableLayout }) {
  return (
    <div
      style={{ gridTemplateColumns: layout.template }}
      className={`${ROW_GRID} border-l-transparent pb-1 text-[9px] font-medium tracking-[0.14em] text-neutral-600 uppercase`}
    >
      {visibleColumns(layout.withSkins).map((column) => (
        <span
          key={column.key}
          title={column.hint}
          className={column.left ? "" : "text-center"}
        >
          {column.label}
        </span>
      ))}
    </div>
  );
}

/** The app: two team blocks, ally first, coloured by `isAlly` only. */
export function PlayerTable({ snapshot }: { snapshot: TrackerSnapshot }) {
  const pregame = snapshot.status === "Pregame";
  const withSkins = snapshot.status === "Ingame";
  const { allies, enemies } = splitTeams(snapshot.players);

  const layout: TableLayout = {
    template: gridTemplate(withSkins),
    withSkins,
    parties: partyGroups(snapshot.players),
  };

  return (
    <div className="min-w-fit px-4 pt-2 pb-3">
      <Legend layout={layout} />
      <TeamBlock
        side="ally"
        label={pregame ? "Your team · agent select" : "Your team"}
        players={allies}
        layout={layout}
      />
      {enemies.length > 0 && (
        <TeamBlock side="enemy" label="Enemy team" players={enemies} layout={layout} />
      )}
      {pregame && (
        <p className="mt-4 pl-2 text-[11px] text-neutral-600">
          Riot hides the enemy team during agent select — they appear when the match starts.
        </p>
      )}
    </div>
  );
}
