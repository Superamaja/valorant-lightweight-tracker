/** The whole backend surface: two commands and one event. See `docs/ipc-contract.md`. */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { TrackerSnapshot } from "./types";

const TRACKER_EVENT = "tracker-state";

/** Starts the background loop. Idempotent — safe to call on every mount. */
export const startTracker = (): Promise<void> => invoke("start_tracker");

/** The current snapshot, for the first paint only. Never poll this. */
export const getTrackerState = (): Promise<TrackerSnapshot> => invoke("get_tracker_state");

/** Subscribes to every resolved state change. */
export const onTrackerState = (
  handle: (snapshot: TrackerSnapshot) => void,
): Promise<UnlistenFn> =>
  listen<TrackerSnapshot>(TRACKER_EVENT, (event) => handle(event.payload));
