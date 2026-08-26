import { useSyncExternalStore } from "react";
import { getUpdateState, subscribeUpdateState, type UpdateState } from "../lib/updater";

/** Reads the shared update-check state, so the header and the status screen agree. */
export function useUpdateState(): UpdateState {
  return useSyncExternalStore(subscribeUpdateState, getUpdateState);
}
