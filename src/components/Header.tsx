import { useEffect, useState } from "react";
import { useUpdateState } from "../hooks/useUpdateState";
import type { AppStatus, TrackerSnapshot } from "../ipc/types";
import { clockTime, relativeTime } from "../lib/format";
import { runUpdateCheck, runUpdateInstall } from "../lib/updater";
import { ArrowLeftIcon, Img } from "./primitives";

/** The chip doubles as the health indicator: a lit dot means the client is connected. */
const CHIP: Record<AppStatus, { label: string; dot: string; pulse: boolean }> = {
  ValorantNotRunning: { label: "Waiting for VALORANT", dot: "bg-neutral-600", pulse: false },
  Menus: { label: "Waiting for match", dot: "bg-accent", pulse: true },
  Pregame: { label: "Agent select", dot: "bg-accent", pulse: true },
  Ingame: { label: "In match", dot: "bg-accent", pulse: false },
};

const CONNECTING = { label: "Starting", dot: "bg-neutral-600", pulse: true };

/** Shown while a finished match's table is still up: the rows are history, not live. */
const LAST_MATCH = { label: "Last match", dot: "bg-neutral-600", pulse: false };

/** How long a live snapshot may go unrefreshed before its age is worth showing. */
const STALE_MS = 90_000;

/**
 * Snapshot age. A live match refreshes itself, so the age is noise there and only appears
 * once the feed has gone quiet; a held last-match table always says how old it is.
 */
function LastUpdated({ at, held }: { at: number; held: boolean }) {
  const [, tick] = useState(0);

  useEffect(() => {
    const timer = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(timer);
  }, []);

  if (!held && Date.now() - at < STALE_MS) return null;

  return (
    <span
      className="text-[10px] tabular-nums text-neutral-600"
      title={`Last update ${clockTime(at)}`}
    >
      {relativeTime(at)}
    </span>
  );
}

/**
 * Nothing at all until a check has found a newer version — the version itself lives on the
 * status screen, so the header only speaks up when there is something to act on. The one
 * check of the session runs from here; clicking installs and restarts.
 */
function UpdateBadge() {
  const { result, installing, installError } = useUpdateState();

  useEffect(() => {
    void runUpdateCheck();
  }, []);

  if (result?.state !== "available") return null;

  const label = installing
    ? "Updating"
    : `${installError ? "Retry" : "Update"}: v${result.version}`;

  return (
    <button
      type="button"
      onClick={runUpdateInstall}
      disabled={installing}
      title={
        installing
          ? "Downloading the update"
          : (installError ?? "Install it and restart")
      }
      className="flex items-center gap-1.5 rounded-full bg-accent/10 px-2.5 py-1 text-[10px] tabular-nums text-accent/80 transition-colors hover:bg-accent/20 hover:text-accent"
    >
      <span
        className={`h-1.5 w-1.5 rounded-full bg-accent ${installing ? "animate-pulse" : ""}`}
      />
      {label}
    </button>
  );
}

/**
 * Map, mode, state chip and freshness. The map splash carries the colour. With `lastMatch`
 * the snapshot is a held one from a match that has ended, so the chip must not read as live.
 */
export function Header({
  snapshot,
  lastMatch = false,
  onLeaveLastMatch,
}: {
  snapshot: TrackerSnapshot | null;
  lastMatch?: boolean;
  /** The way back out of the held table; only meaningful alongside `lastMatch`. */
  onLeaveLastMatch?: () => void;
}) {
  const map = snapshot?.map ?? null;
  const chip = lastMatch ? LAST_MATCH : snapshot ? CHIP[snapshot.status] : CONNECTING;
  const mode = snapshot?.mode ?? null;

  return (
    <header className="relative shrink-0 overflow-hidden border-b border-edge">
      {map?.splashUrl && (
        <>
          <div
            className="absolute inset-0 bg-cover bg-center opacity-25"
            style={{ backgroundImage: `url("${map.splashUrl}")` }}
          />
          <div className="absolute inset-0 bg-linear-to-r from-base via-base/80 to-base/95" />
        </>
      )}

      <div className="relative flex items-center gap-3 px-4 py-2.5">
        {map && (
          <div className="h-9 w-14 shrink-0 overflow-hidden rounded-sm ring-1 ring-white/10">
            <Img
              src={map.listViewUrl}
              alt={map.name}
              className="h-full w-full object-cover"
              fallback={<span className="block h-full w-full bg-white/5" />}
            />
          </div>
        )}

        <div className="min-w-0">
          <h1 className="truncate text-[13px] font-semibold tracking-[0.14em] text-neutral-100 uppercase">
            {map?.name || "Valorant Tracker"}
          </h1>
          {mode && <p className="truncate text-[11px] text-neutral-500">{mode}</p>}
        </div>

        <div className="ml-auto flex shrink-0 items-center gap-3">
          <UpdateBadge />
          {lastMatch && onLeaveLastMatch && (
            <button
              type="button"
              onClick={onLeaveLastMatch}
              className="flex items-center gap-1 text-[10px] tracking-wide text-neutral-500 transition-colors hover:text-neutral-200"
            >
              <ArrowLeftIcon />
              Back
            </button>
          )}
          <span className="flex items-center gap-1.5 rounded-full bg-white/5 px-2.5 py-1 text-[10px] tracking-wide text-neutral-300">
            <span
              className={`h-1.5 w-1.5 rounded-full ${chip.dot} ${chip.pulse ? "animate-pulse" : ""}`}
            />
            {chip.label}
          </span>
          {snapshot && <LastUpdated at={snapshot.lastUpdated} held={lastMatch} />}
        </div>
      </div>
    </header>
  );
}
