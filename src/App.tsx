import { useEffect, useRef } from "react";
import { Header } from "./components/Header";
import { PlayerTable } from "./components/PlayerTable";
import { StatusScreen } from "./components/StatusScreen";
import { useTracker } from "./hooks/useTracker";
import type { TrackerSnapshot } from "./ipc/types";

/**
 * Status drives the whole layout — the table only exists inside a match, except in `Menus`,
 * where `lastMatch` (the match we just left, if any) keeps its table on screen.
 */
function screen(
  snapshot: TrackerSnapshot | null,
  error: string | null,
  lastMatch: TrackerSnapshot | null,
) {
  if (error) {
    return <StatusScreen tone="error" title="The tracker stopped" subtitle={error} />;
  }
  if (!snapshot) {
    return (
      <StatusScreen tone="live" title="Starting" subtitle="Looking for the Valorant client." />
    );
  }

  switch (snapshot.status) {
    case "ValorantNotRunning":
      return (
        <StatusScreen
          tone="idle"
          title="Waiting for VALORANT"
          subtitle={snapshot.message ?? "Start the game — the tracker connects on its own."}
        />
      );
    case "Menus":
      return lastMatch ? (
        <PlayerTable snapshot={lastMatch} />
      ) : (
        <StatusScreen
          tone="live"
          title="Waiting for a match"
          subtitle={
            snapshot.message ?? "Connected. The table fills in the moment agent select opens."
          }
        />
      );
    default:
      return snapshot.players.length > 0 ? (
        <PlayerTable snapshot={snapshot} />
      ) : (
        <StatusScreen tone="live" title="Loading the lobby" subtitle={snapshot.message} />
      );
  }
}

export default function App() {
  const { snapshot, error } = useTracker();

  // The last match we saw this session, kept so leaving a match does not blank the table.
  // A ref written after commit (render must stay pure); by the time a `Menus` snapshot
  // renders, the previous match's committed snapshot is already in it.
  const seen = useRef<TrackerSnapshot | null>(null);
  useEffect(() => {
    if (snapshot && snapshot.players.length > 0) {
      seen.current = snapshot;
    }
  }, [snapshot]);
  // Only plain `Menus` holds it — not running, error and startup keep their status screens.
  const lastMatch = snapshot?.status === "Menus" ? seen.current : null;

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-base text-neutral-200 select-none">
      <Header snapshot={lastMatch ?? snapshot} lastMatch={lastMatch !== null} />
      <main className="min-h-0 flex-1 overflow-auto">{screen(snapshot, error, lastMatch)}</main>
    </div>
  );
}
