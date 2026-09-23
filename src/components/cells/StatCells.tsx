import type { ReactNode } from "react";
import { STATS_WINDOW, type MatchResult, type WinRate } from "../../ipc/types";
import { signed } from "../../lib/format";
import { Skeleton } from "../primitives";
import { Tooltip } from "../Tooltip";

/**
 * One compact stat. While its group is still in flight an absent value is a skeleton; once
 * the group has settled the same absence means the player has no data.
 */
function Stat({
  missing,
  pending,
  title,
  className = "text-neutral-300",
  children,
}: {
  missing: boolean;
  pending: boolean;
  /** Tooltip; `\n` breaks a line. */
  title: string;
  className?: string;
  children?: ReactNode;
}) {
  if (missing) {
    return pending ? (
      <span className="flex justify-center">
        <Skeleton className="h-1.5 w-7 rounded-full" />
      </span>
    ) : (
      <Tooltip content={title}>
        <span className="text-center text-[11px] text-neutral-700">N/A</span>
      </Tooltip>
    );
  }
  return (
    <Tooltip content={title}>
      <span className={`text-center text-[12px] ${className}`}>{children}</span>
    </Tooltip>
  );
}

/** The matches HS% and KD stand on, next to how many recent matches the player has. */
export interface StatsScope {
  /** `statsMatches`; null when unknown, which reads as a full window. */
  matches: number | null;
  results: number;
}

/** Fewer matches than a full window: still a real number, just a thinner one. */
const isPartial = ({ matches }: StatsScope) => matches !== null && matches < STATS_WINDOW;

const scopeTone = (scope: StatsScope) =>
  `tabular-nums ${isPartial(scope) ? "text-neutral-500" : "text-neutral-300"}`;

function scopeNote({ matches, results }: StatsScope): string {
  if (matches === null || matches >= STATS_WINDOW) {
    return `Last ${STATS_WINDOW} competitive matches`;
  }
  if (matches < results) {
    return `Based on ${matches} of the last ${results} matches (details for the rest aren't available yet)`;
  }
  return matches === 1
    ? "Based on their only recent competitive match"
    : `Based on the only ${matches} recent competitive matches`;
}

export function HeadshotCell({
  percent,
  scope,
  pending,
}: {
  percent: number | null;
  scope: StatsScope;
  pending: boolean;
}) {
  return (
    <Stat
      missing={percent === null}
      pending={pending}
      className={scopeTone(scope)}
      title={percent === null ? "No recent competitive matches" : `Headshot %\n${scopeNote(scope)}`}
    >
      {percent}%
    </Stat>
  );
}

export function KdCell({
  kd,
  scope,
  pending,
}: {
  kd: number | null;
  scope: StatsScope;
  pending: boolean;
}) {
  // == also catches undefined from a pre-`kd` debug snapshot; a strict check would throw
  // on toFixed and blank the whole app.
  return (
    <Stat
      missing={kd == null}
      pending={pending}
      className={scopeTone(scope)}
      title={kd == null ? "No recent competitive matches" : `Kills/deaths\n${scopeNote(scope)}`}
    >
      {kd == null ? null : kd.toFixed(2)}
    </Stat>
  );
}

/** Win rate rides the MMR payload, so it settles with the rank cells. */
export function WinRateCell({
  winRate,
  pending,
}: {
  winRate: WinRate | null;
  pending: boolean;
}) {
  return (
    <Stat
      missing={winRate === null}
      pending={pending}
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

export function RrChangeCell({ change, pending }: { change: number | null; pending: boolean }) {
  const tone = change === null || change === 0 ? "text-neutral-400" : change > 0 ? "text-win" : "text-loss";
  return (
    <Stat
      missing={change === null}
      pending={pending}
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
  Draw: "bg-amber-300/70",
  Unknown: "bg-neutral-600",
};
const SLOTS = [0, 1, 2, 3, 4];

/** Last five competitive results, newest first. Empty segments keep the column aligned. */
export function ResultPips({ results, pending }: { results: MatchResult[]; pending: boolean }) {
  const empty = results.length === 0;
  const title = empty
    ? pending
      ? "Loading recent matches"
      : "No recent competitive matches"
    : `Last ${results.length}, newest first\n${results.join(" · ")}`;

  if (empty && pending) {
    return (
      <span className="flex justify-center">
        <Skeleton className="h-1.5 w-14 rounded-full" />
      </span>
    );
  }

  return (
    <Tooltip content={title}>
      <span className="flex justify-center">
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
    </Tooltip>
  );
}
