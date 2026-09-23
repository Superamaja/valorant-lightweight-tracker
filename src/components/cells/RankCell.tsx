import type { RankInfo } from "../../ipc/types";
import { Img, Skeleton } from "../primitives";
import { Tooltip } from "../Tooltip";

/** Current and peak rank share one icon size so the pair reads as equals. */
const ICON = "h-8 w-8";

/**
 * A rank cell whose MMR has not landed. Until it does the row has no rank at all — showing
 * the assembler's placeholder Unranked would be a wrong answer rather than a missing one.
 */
function RankSkeleton({ size }: { size: string }) {
  return (
    <Tooltip content="Loading rank">
      <div className="flex items-center gap-1.5">
        <Skeleton className={`${size} shrink-0 rounded-full`} />
        <Skeleton className="h-1.5 w-6 rounded-full" />
      </div>
    </Tooltip>
  );
}

function RankIcon({ rank, size }: { rank: RankInfo; size: string }) {
  return (
    <Img
      src={rank.iconUrl}
      alt={rank.name}
      className={`${size} shrink-0 object-contain`}
      fallback={<span className={`${size} shrink-0 rounded-full border border-dashed border-white/10`} />}
    />
  );
}

/** Current rank: icon first, RR next to it, rank name only as a tooltip. */
export function RankCell({
  rank,
  rr,
  leaderboardRank,
  pending,
}: {
  rank: RankInfo;
  rr: number;
  leaderboardRank: number;
  pending: boolean;
}) {
  if (pending) return <RankSkeleton size={ICON} />;

  const ranked = rank.tier > 0;
  const title = ranked
    ? `${rank.name} · ${rr} RR${leaderboardRank > 0 ? ` · Leaderboard #${leaderboardRank}` : ""}`
    : rank.name;

  // Unranked has no rating to show, so the icon carries the cell on its own.
  return (
    <Tooltip content={title}>
      <div className="flex items-center gap-1.5">
        <RankIcon rank={rank} size={ICON} />
        {ranked && (
          <span className="flex flex-col leading-none">
            <span className="text-[12px] tabular-nums text-neutral-200">
              {rr}
              <span className="text-[9px] text-neutral-500"> RR</span>
            </span>
            {leaderboardRank > 0 && (
              <span className="mt-0.5 text-[9px] tabular-nums text-accent">#{leaderboardRank}</span>
            )}
          </span>
        )}
      </div>
    </Tooltip>
  );
}

/** Peak rank: full colour with the act it was set in. */
export function PeakCell({
  rank,
  act,
  pending,
}: {
  rank: RankInfo;
  act: string | null;
  pending: boolean;
}) {
  if (pending) return <RankSkeleton size={ICON} />;

  const title = `Peak: ${rank.name}${act ? ` · ${act}` : ""}`;
  return (
    <Tooltip content={title}>
      <div className="flex items-center gap-1.5">
        <RankIcon rank={rank} size={ICON} />
        {act && <span className="text-[9px] tracking-wide text-neutral-500">{act}</span>}
      </div>
    </Tooltip>
  );
}
