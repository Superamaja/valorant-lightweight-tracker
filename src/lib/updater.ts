/** Version reporting and the auto-updater's frontend half. */

import { invoke } from "@tauri-apps/api/core";

/** Baked in at build time from package.json. */
export const APP_VERSION = __APP_VERSION__;

export type UpdateCheck =
  | { state: "upToDate" }
  | { state: "available"; version: string }
  | { state: "error" };

/** What the backend's `check_update` answers with. */
interface UpdateInfo {
  available: boolean;
  version: string;
}

export async function checkForUpdates(): Promise<UpdateCheck> {
  const { available, version } = await invoke<UpdateInfo>("check_update");
  return available ? { state: "available", version } : { state: "upToDate" };
}

/**
 * Installs the latest release and restarts into it — on success the app quits, so this
 * only ever returns by throwing.
 */
export const applyUpdate = (): Promise<void> => invoke("apply_update");

export interface UpdateState {
  checking: boolean;
  /** An install is downloading; the app restarts itself when it lands. */
  installing: boolean;
  /** The most recent finished check, kept so any screen can report it. */
  result: UpdateCheck | null;
  /** Why the last install failed, in the backend's words; null once another one starts. */
  installError: string | null;
}

const IDLE: UpdateState = {
  checking: false,
  installing: false,
  result: null,
  installError: null,
};

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

/** The backend rejects with a display-ready message; anything else is not worth showing. */
function failureText(error: unknown): string {
  return typeof error === "string" ? error : "The update could not be installed.";
}

/**
 * The one way to run a check: the result is shared, so every screen sees the same answer.
 * An install is already on its way to a restart, so a check during one has nothing to add.
 */
export async function runUpdateCheck(): Promise<void> {
  if (state.checking || state.installing) return;
  publish({ ...state, checking: true, result: null });
  try {
    const result = await checkForUpdates();
    publish({ ...state, checking: false, result });
  } catch {
    publish({ ...state, checking: false, result: { state: "error" } });
  }
}

/**
 * The one way to install. A failure keeps the check's result standing, so the update stays
 * on offer and the user can try again.
 */
export async function runUpdateInstall(): Promise<void> {
  if (state.installing || state.checking) return;
  publish({ ...state, installing: true, installError: null });
  try {
    await applyUpdate();
  } catch (error) {
    publish({ ...state, installing: false, installError: failureText(error) });
  }
}
