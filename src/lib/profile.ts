import { openUrl } from "@tauri-apps/plugin-opener";

/** tracker.gg is a link-out only — no scraping, no API. See `docs/ui-spec.md`. */
const PROFILE_BASE = "https://tracker.gg/valorant/profile/riot";

const profileUrl = (riotId: string): string =>
  `${PROFILE_BASE}/${encodeURIComponent(riotId)}/overview`;

/** Opens a "name#tag" profile in the default browser. */
export async function openProfile(riotId: string): Promise<void> {
  try {
    await openUrl(profileUrl(riotId));
  } catch (cause) {
    console.error("Could not open the tracker.gg profile", cause);
  }
}

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
