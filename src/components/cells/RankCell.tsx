import type { RankInfo } from "../../ipc/types";
import { Img, Skeleton } from "../primitives";

const ICON = "h-7 w-7";

/**
 * A rank cell whose MMR has not landed. Until it does the row has no rank at all — showing
 * the assembler's placeholder Unranked would be a wrong answer rather than a missing one.
 */
function RankSkeleton() {
  return (
    <div className="flex items-center gap-1.5" title="Loading rank">
      <Skeleton className={`${ICON} shrink-0 rounded-full`} />
      <Skeleton className="h-1.5 w-6 rounded-full" />
    </div>
  );
}

function RankIcon({ rank, size, title }: { rank: RankInfo; size: string; title: string }) {
  return (
    <Img
      src={rank.iconUrl}
      alt={rank.name}
      title={title}
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
  if (pending) return <RankSkeleton />;

  const ranked = rank.tier > 0;
  const title = ranked
    ? `${rank.name} · ${rr} RR${leaderboardRank > 0 ? ` · leaderboard #${leaderboardRank}` : ""}`
    : rank.name;

  return (
    <div className="flex items-center gap-1.5" title={title}>
      <RankIcon rank={rank} size={ICON} title={title} />
      <span className="flex flex-col leading-none">
        {ranked ? (
          <span className="text-[12px] tabular-nums text-neutral-200">
            {rr}
            <span className="text-[9px] text-neutral-500"> RR</span>
          </span>
        ) : (
          <span className="text-[12px] text-neutral-700">—</span>
        )}
        {leaderboardRank > 0 && (
          <span className="mt-0.5 text-[9px] tabular-nums text-accent">#{leaderboardRank}</span>
        )}
      </span>
    </div>
  );
}

/** Peak rank: same icon size and weight as the current rank, with the act it was set in. */
export function PeakCell({
  rank,
  act,
  pending,
}: {
  rank: RankInfo;
  act: string | null;
  pending: boolean;
}) {
  if (pending) return <RankSkeleton />;

  const title = `Peak: ${rank.name}${act ? ` · ${act}` : ""}`;
  return (
    <div className="flex items-center gap-1.5" title={title}>
      <RankIcon rank={rank} size={ICON} title={title} />
      {act && <span className="text-[9px] tracking-wide text-neutral-500">{act}</span>}
    </div>
  );
}
