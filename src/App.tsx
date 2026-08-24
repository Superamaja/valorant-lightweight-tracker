import { Header } from "./components/Header";
import { PlayerTable } from "./components/PlayerTable";
import { StatusScreen } from "./components/StatusScreen";
import { useTracker } from "./hooks/useTracker";
import type { TrackerSnapshot } from "./ipc/types";

/** Status drives the whole layout — the table only exists inside a match. */
function screen(snapshot: TrackerSnapshot | null, error: string | null) {
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
      return (
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

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-base text-neutral-200 select-none">
      <Header snapshot={snapshot} />
      <main className="min-h-0 flex-1 overflow-auto">{screen(snapshot, error)}</main>
    </div>
  );
}
