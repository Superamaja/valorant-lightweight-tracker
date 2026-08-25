import { useEffect, useState } from "react";
import { getTrackerState, onTrackerState, startTracker } from "../ipc/tracker";
import type { TrackerSnapshot } from "../ipc/types";

const DEBUG_SNAPSHOT_URL = "/debug-snapshot.json";

/**
 * Dev-only: a snapshot placed at `public/debug-snapshot.json` (see `docs/testing.md`) lets
 * the UI be driven from a plain browser, where the Tauri APIs do not exist. Vite answers a
 * missing public file with the `index.html` fallback rather than a 404, so a non-JSON
 * content type — or a `.json()` that throws on the HTML body — means "no debug snapshot".
 * Returns null in every such case so the caller falls through to the normal IPC path.
 */
async function loadDebugSnapshot(): Promise<TrackerSnapshot | null> {
  try {
    const response = await fetch(DEBUG_SNAPSHOT_URL);
    if (!response.ok) return null;
    if (!response.headers.get("content-type")?.includes("json")) return null;
    return (await response.json()) as TrackerSnapshot;
  } catch {
    return null;
  }
}

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
      // Stripped from production builds: `import.meta.env.DEV` is a compile-time false there,
      // so the branch — and `loadDebugSnapshot` with it — is tree-shaken away.
      if (import.meta.env.DEV) {
        const debug = await loadDebugSnapshot();
        if (debug) {
          if (!stopped) setSnapshot(debug);
          return; // no invoke, no listen — the browser has no Tauri APIs
        }
      }
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
