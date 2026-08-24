import { useEffect, useState } from "react";
import type { PlayerRow } from "../../ipc/types";
import { displayName } from "../../lib/players";
import { copyText, openProfile } from "../../lib/profile";
import { CheckIcon, CopyIcon } from "../primitives";

/**
 * Riot id. Click opens tracker.gg, right-click (or the hover icon) copies it. Both are
 * disabled for incognito players, who are shown by agent instead.
 */
export function NameCell({ player }: { player: PlayerRow }) {
  const [copied, setCopied] = useState(false);
  const riotId = player.name;
  const label = displayName(player);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1400);
    return () => clearTimeout(timer);
  }, [copied]);

  const copy = (id: string) => {
    void copyText(id).then((ok) => ok && setCopied(true));
  };

  return (
    <div className="flex min-w-0 items-center gap-1.5">
      {riotId ? (
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
          className="min-w-0 truncate text-[13px] text-neutral-500 italic"
        >
          {label}
        </span>
      )}

      {player.isSelf && (
        <span className="shrink-0 rounded-sm bg-accent/15 px-1 py-px text-[9px] font-semibold tracking-wider text-accent">
          YOU
        </span>
      )}

      {riotId && (
        <button
          type="button"
          onClick={() => copy(riotId)}
          title="Copy name#tag"
          className={`shrink-0 cursor-pointer transition-opacity focus-visible:outline-none ${
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
