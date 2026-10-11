import { SELECTED, type AgentInfo } from "../../ipc/types";
import { groupTint } from "../../lib/table";
import { Img } from "../primitives";
import { Tooltip } from "../Tooltip";

/**
 * Agent portrait. In pregame it also carries the pick state: pulsing = still choosing.
 * The account level rides the portrait's bottom-right corner — it has no column of its own.
 * A hidden level (incognito, "hide my level", or Riot's zeroed wire value) arrives as null
 * from the backend and simply renders nothing.
 * In a multi-team mode whose players all share a stand-in agent, the portrait takes its
 * group's colour, as the in-game scoreboard does.
 */
export function AgentCell({
  agent,
  group,
  selectionState,
  level,
}: {
  agent: AgentInfo | null;
  group: number | null;
  selectionState: string | null;
  level: number | null;
}) {
  const tint =
    group !== null && agent?.playable === false && agent.iconUrl ? groupTint(group) : null;
  const picking = agent === null && selectionState === SELECTED;
  // Hovered but not locked in — the portrait shows dimmed so a tentative pick reads as such.
  // A locked pick needs no marker of its own: it simply renders like any in-game agent.
  const hovered = agent !== null && selectionState === SELECTED;
  const label = agent
    ? hovered
      ? `${agent.name} (not locked)`
      : agent.name
    : picking
      ? "Picking an agent"
      : "No agent yet";

  return (
    <div className="relative h-10 w-10">
      <Tooltip content={label}>
        <div
          className={`relative isolate h-full w-full overflow-hidden rounded-md bg-white/[0.03] ring-1 ring-white/10 ${
            picking ? "animate-pulse" : ""
          } ${hovered ? "opacity-60 saturate-50" : ""}`}
        >
          <Img
            src={agent?.iconUrl ?? null}
            alt={label}
            className="h-full w-full object-cover"
            fallback={
              <span className="flex h-full w-full items-center justify-center text-[11px] text-neutral-700">
                ?
              </span>
            }
          />
          {tint && (
            <span
              className="pointer-events-none absolute inset-0 mix-blend-color"
              style={{ backgroundColor: tint }}
            />
          )}
        </div>
      </Tooltip>
      {level !== null && level > 0 && (
        <Tooltip content={`Account level ${level}`}>
          <span className="absolute -right-1 -bottom-1 rounded-full bg-black/85 px-1 text-[10px] leading-[15px] font-medium tabular-nums text-neutral-300 ring-1 ring-white/15">
            {level}
          </span>
        </Tooltip>
      )}
    </div>
  );
}
