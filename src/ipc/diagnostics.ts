/** The release-build diagnostics report. See `docs/ipc-contract.md`. */

import { invoke } from "@tauri-apps/api/core";

/** What the frontend knows about the current view and the backend does not. */
export interface UiFacts {
  /** The screen the user is looking at, in the words the screen itself uses. */
  screen: string;
  /** Whether those rows are the held last-match table rather than a live one. */
  heldTable: boolean;
}

/**
 * A preformatted plain-text report of what the tracker last saw at each stage, ready to
 * paste into a GitHub issue. The format is the backend's business, not a contract.
 */
export const getDiagnostics = (ui: UiFacts): Promise<string> =>
  invoke("get_diagnostics", { ui });
