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
