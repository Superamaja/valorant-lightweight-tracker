import { openUrl } from "@tauri-apps/plugin-opener";
import { APP_VERSION } from "./updater";

/** tracker.gg is a link-out only — no scraping, no API. See `docs/ui-spec.md`. */
const PROFILE_BASE = "https://tracker.gg/valorant/profile/riot";
const REPO = "https://github.com/Superamaja/valorant-lightweight-tracker";

const profileUrl = (riotId: string): string =>
  `${PROFILE_BASE}/${encodeURIComponent(riotId)}/overview`;

/** The bug form with the version filled in. The diagnostics report travels by clipboard, never by URL. */
export const bugReportUrl = (version: string): string =>
  `${REPO}/issues/new?${new URLSearchParams({ template: "bug_report.yml", version: `v${version}` })}`;

async function openExternal(url: string, failure: string): Promise<void> {
  try {
    await openUrl(url);
  } catch (cause) {
    console.error(failure, cause);
  }
}

/** Opens a "name#tag" profile in the default browser. */
export const openProfile = (riotId: string): Promise<void> =>
  openExternal(profileUrl(riotId), "Could not open the tracker.gg profile");

/** Opens the repo's bug form in the default browser. */
export const openBugReport = (): Promise<void> =>
  openExternal(bugReportUrl(APP_VERSION), "Could not open the bug report page");

/** Copies text, falling back to the legacy path when the clipboard API is unavailable. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return legacyCopy(text);
  }
}

function legacyCopy(text: string): boolean {
  const field = document.createElement("textarea");
  field.value = text;
  field.setAttribute("readonly", "");
  field.style.position = "fixed";
  field.style.opacity = "0";
  document.body.appendChild(field);
  field.select();
  const copied = document.execCommand("copy");
  field.remove();
  return copied;
}
