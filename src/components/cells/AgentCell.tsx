import { LOCKED, SELECTED, type AgentInfo } from "../../ipc/types";
import { Img } from "../primitives";

/** Agent portrait. In pregame it also carries the pick state: pulsing = still choosing. */
export function AgentCell({
  agent,
  selectionState,
}: {
  agent: AgentInfo | null;
  selectionState: string | null;
}) {
  const picking = agent === null && selectionState === SELECTED;
  const locked = agent !== null && selectionState === LOCKED;
  const label = agent?.name || (picking ? "Picking an agent" : "No agent yet");

  return (
    <div
      title={label}
      className={`h-9 w-9 overflow-hidden rounded-md bg-white/[0.03] ring-1 ${
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
  );
}
