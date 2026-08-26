/** Version reporting and the seam the auto-updater will grow into. */

/** Baked in at build time from package.json. */
export const APP_VERSION = __APP_VERSION__;

export type UpdateCheck =
  | { state: "upToDate" }
  | { state: "available"; version: string }
  | { state: "error" };

/**
 * There is no auto-updater yet, so every check answers "up to date". Wiring one up replaces
 * this function's body; nothing else in the UI has to change.
 */
export async function checkForUpdates(): Promise<UpdateCheck> {
  return { state: "upToDate" };
}

export interface UpdateState {
  checking: boolean;
  /** The most recent finished check, kept so any screen can report it. */
  result: UpdateCheck | null;
}

const IDLE: UpdateState = { checking: false, result: null };

let state = IDLE;
const listeners = new Set<() => void>();

function publish(next: UpdateState) {
  state = next;
  for (const listener of listeners) listener();
}

export function getUpdateState(): UpdateState {
  return state;
}

export function subscribeUpdateState(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The one way to run a check: the result is shared, so every screen sees the same answer. */
export async function runUpdateCheck(): Promise<void> {
  if (state.checking) return;
  publish({ checking: true, result: null });
  try {
    publish({ checking: false, result: await checkForUpdates() });
  } catch {
    publish({ checking: false, result: { state: "error" } });
  }
}
