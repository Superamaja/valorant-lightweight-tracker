import { LOCKED, SELECTED, type AgentInfo } from "../../ipc/types";
import { Img } from "../primitives";

/**
 * Agent portrait. In pregame it also carries the pick state: pulsing = still choosing.
 * The account level rides the portrait's bottom-right corner — it has no column of its own.
 * A hidden level (incognito, "hide my level", or Riot's zeroed wire value) arrives as null
 * from the backend and simply renders nothing.
 */
export function AgentCell({
  agent,
  selectionState,
  level,
}: {
  agent: AgentInfo | null;
  selectionState: string | null;
  level: number | null;
}) {
  const picking = agent === null && selectionState === SELECTED;
  const locked = agent !== null && selectionState === LOCKED;
  const label = agent?.name || (picking ? "Picking an agent" : "No agent yet");

  return (
    <div className="relative h-10 w-10">
      <div
        title={label}
        className={`h-full w-full overflow-hidden rounded-md bg-white/[0.03] ring-1 ${
          locked ? "ring-accent/40" : "ring-white/10"
        } ${picking ? "animate-pulse" : ""}`}
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
      </div>
      {level !== null && level > 0 && (
        <span
          title={`Account level ${level}`}
          className="absolute -right-1 -bottom-1 rounded-full bg-black/85 px-1 text-[10px] leading-[15px] font-medium tabular-nums text-neutral-300 ring-1 ring-white/15"
        >
          {level}
        </span>
      )}
    </div>
  );
}
