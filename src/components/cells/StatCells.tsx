import type { ReactNode } from "react";
import type { MatchResult, WinRate } from "../../ipc/types";
import { signed } from "../../lib/format";
import { HS_SCOPE } from "../../lib/table";
import { Skeleton } from "../primitives";

/**
 * One compact stat. While the enriched snapshot is in flight an absent value is a
 * skeleton; once it has landed the same absence means the player has no data.
 */
function Stat({
  missing,
  loading,
  title,
  className = "text-neutral-300",
  children,
}: {
  missing: boolean;
  loading: boolean;
  title: string;
  className?: string;
  children?: ReactNode;
}) {
  if (missing) {
    return loading ? (
      <span className="flex justify-center">
        <Skeleton className="h-1.5 w-7 rounded-full" />
      </span>
    ) : (
      <span className="text-center text-[11px] text-neutral-700" title={title}>
        N/A
      </span>
    );
  }
  return (
    <span className={`text-center text-[12px] ${className}`} title={title}>
      {children}
    </span>
  );
}

export function HeadshotCell({ percent, loading }: { percent: number | null; loading: boolean }) {
  return (
    <Stat
      missing={percent === null}
      loading={loading}
      className="tabular-nums text-neutral-300"
      title={
        percent === null
          ? "No recent competitive matches"
          : `Headshot % over the ${HS_SCOPE}`
      }
    >
      {percent}%
    </Stat>
  );
}

export function KdCell({ kd, loading }: { kd: number | null; loading: boolean }) {
  // == also catches undefined from a pre-`kd` debug snapshot; a strict check would throw
  // on toFixed and blank the whole app.
  return (
    <Stat
      missing={kd == null}
      loading={loading}
      className="tabular-nums text-neutral-300"
      title={
        kd == null ? "No recent competitive matches" : `Kills/deaths over the ${HS_SCOPE}`
      }
    >
      {kd == null ? null : kd.toFixed(2)}
    </Stat>
  );
}

/** Win rate ships with the fast snapshot, so it never shows a skeleton. */
export function WinRateCell({ winRate }: { winRate: WinRate | null }) {
  return (
    <Stat
      missing={winRate === null}
      loading={false}
      className="tabular-nums text-neutral-300"
      title={
        winRate === null
          ? "No competitive games this season"
          : `${winRate.percent}% over ${winRate.games} competitive games this season`
      }
    >
      {winRate && (
        <>
          {winRate.percent}%
          <span className="ml-1 text-[10px] text-neutral-600">({winRate.games})</span>
        </>
      )}
    </Stat>
  );
}

export function RrChangeCell({ change, loading }: { change: number | null; loading: boolean }) {
  const tone = change === null || change === 0 ? "text-neutral-400" : change > 0 ? "text-win" : "text-loss";
  return (
    <Stat
      missing={change === null}
      loading={loading}
      className={`tabular-nums ${tone}`}
      title={change === null ? "No recent competitive match" : "RR from their last competitive match"}
    >
      {change === null ? null : signed(change)}
    </Stat>
  );
}

/** The whole bar. `gap-px` lets the container's colour show through as the segment splits. */
const BAR = "flex h-1.5 w-14 gap-px overflow-hidden rounded-full bg-base";
const PIP_TONE: Record<MatchResult, string> = {
  Win: "bg-win",
  Loss: "bg-loss",
  Unknown: "bg-neutral-600",
};
const PIP_LETTER: Record<MatchResult, string> = { Win: "W", Loss: "L", Unknown: "?" };
const SLOTS = [0, 1, 2, 3, 4];

/** Last five competitive results, newest first. Empty segments keep the column aligned. */
export function ResultPips({ results, loading }: { results: MatchResult[]; loading: boolean }) {
  const empty = results.length === 0;
  const title = empty
    ? loading
      ? "Loading recent matches"
      : "No recent competitive matches"
    : `Last ${results.length}, newest first: ${results.map((r) => PIP_LETTER[r]).join(" ")}`;

  if (empty && loading) {
    return (
      <span className="flex justify-center">
        <Skeleton className="h-1.5 w-14 rounded-full" />
      </span>
    );
  }

  return (
    <span className="flex justify-center" title={title}>
      <span className={BAR}>
        {SLOTS.map((slot) => {
          const result = results[slot];
          return (
            <span
              key={slot}
              className={`flex-1 ${result ? PIP_TONE[result] : "bg-white/5"}`}
            />
          );
        })}
      </span>
    </span>
  );
}
