import { useEffect, useState } from "react";
import { getTrackerState, onTrackerState, startTracker } from "../ipc/tracker";
import type { TrackerSnapshot } from "../ipc/types";

/**
 * The app's single data source: starts the tracker, paints the current snapshot, then
 * follows the `tracker-state` event. `snapshot` is null only before the first response.
 */
export function useTracker(): { snapshot: TrackerSnapshot | null; error: string | null } {
  const [snapshot, setSnapshot] = useState<TrackerSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let stopped = false;
    let unlisten: (() => void) | undefined;

    // The initial fetch and the first event race each other; the older one must lose.
    const apply = (next: TrackerSnapshot) =>
      setSnapshot((prev) => (prev && prev.lastUpdated > next.lastUpdated ? prev : next));

    void (async () => {
      try {
        await startTracker();
        const stop = await onTrackerState((next) => {
          if (!stopped) apply(next);
        });
        if (stopped) {
          stop();
          return;
        }
        unlisten = stop;
        apply(await getTrackerState());
      } catch (cause) {
        if (!stopped) setError(cause instanceof Error ? cause.message : String(cause));
      }
    })();

    return () => {
      stopped = true;
      unlisten?.();
    };
  }, []);

  return { snapshot, error };
}
