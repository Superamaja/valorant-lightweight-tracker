import { useEffect, useState } from "react";
import type { PlayerRow } from "../../ipc/types";
import { displayName, pendingOf } from "../../lib/players";
import { copyText, openProfile } from "../../lib/profile";
import { CheckIcon, CopyIcon, Skeleton } from "../primitives";

/**
 * Riot id. Click opens tracker.gg, right-click (or the hover icon) copies it. Both are
 * disabled for incognito players, who are shown by agent instead.
 */
export function NameCell({ player }: { player: PlayerRow }) {
  const [copied, setCopied] = useState(false);
  const riotId = player.name;
  const label = displayName(player);
  // Incognito rows are shown by agent and never resolve a name, so only the rows whose name
  // is genuinely still coming wait for it.
  const awaitingName = !riotId && !player.incognito && pendingOf(player).name;

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1400);
    return () => clearTimeout(timer);
  }, [copied]);

  const copy = (id: string) => {
    void copyText(id).then((ok) => ok && setCopied(true));
  };

  return (
    <div className="relative flex min-w-0 items-center gap-1.5">
      {awaitingName ? (
        <Skeleton className="h-2 w-24 rounded-full" />
      ) : riotId ? (
        <button
          type="button"
          onClick={() => void openProfile(riotId)}
          onContextMenu={(event) => {
            event.preventDefault();
            copy(riotId);
          }}
          title={`${riotId}\nClick: tracker.gg profile · Right-click: copy`}
          className="min-w-0 cursor-pointer truncate text-left text-[13px] text-neutral-100 underline-offset-2 select-text hover:text-accent hover:underline focus-visible:text-accent focus-visible:outline-none"
        >
          {label}
        </button>
      ) : (
        <span
          title={player.incognito ? "Streamer mode — name hidden" : "Name unavailable"}
          // Italic marks a placeholder ("Hidden player"); once an agent is known the label is
          // that agent's name — still muted, but no longer a stand-in worth flagging.
          className={`min-w-0 truncate text-[13px] text-neutral-500 ${player.agent ? "" : "italic"}`}
        >
          {label}
        </span>
      )}

      {player.isSelf && (
        <span className="shrink-0 rounded-sm bg-accent/15 px-1 py-px text-[9px] font-semibold tracking-wider text-accent">
          YOU
        </span>
      )}

      {/*
        Out of flow on purpose: inline it reserved its width plus a gap even while invisible,
        which pushed the name into an ellipsis on rows that also carry the YOU badge. Now the
        name and the badge get the whole cell and only truncate when they genuinely overflow.
        Because it can land over the tail of a long name, it fades in as its own opaque chip
        rather than as a bare glyph on top of text — reserving space for it again would bring
        the truncation back, and padding it in only on hover would make the name jump.
      */}
      {riotId && (
        <button
          type="button"
          onClick={() => copy(riotId)}
          title="Copy name#tag"
          className={`absolute top-1/2 right-0 z-10 -translate-y-1/2 cursor-pointer rounded-sm bg-edge/95 p-0.5 ring-1 ring-white/10 backdrop-blur-[2px] transition-opacity focus-visible:outline-none ${
            copied
              ? "text-win opacity-100"
              : "text-neutral-500 opacity-0 hover:text-neutral-200 group-hover:opacity-100 focus-visible:opacity-100"
          }`}
        >
          {copied ? <CheckIcon /> : <CopyIcon />}
        </button>
      )}
    </div>
  );
}
