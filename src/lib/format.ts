/** Small display formatters shared across cells. */

/** "just now" / "42s ago" / "3m ago" / "2h ago" — for the header's health line. */
export function relativeTime(epochMs: number, now: number = Date.now()): string {
  const seconds = Math.max(0, Math.round((now - epochMs) / 1000));
  if (seconds < 5) return "just now";
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  return `${Math.floor(minutes / 60)}h ago`;
}

export const clockTime = (epochMs: number): string => new Date(epochMs).toLocaleTimeString();

/** "+18" / "-14" / "0" */
export const signed = (value: number): string => (value > 0 ? `+${value}` : String(value));
