import { useEffect, useState } from "react";
import { getDiagnostics, type UiFacts } from "../ipc/diagnostics";
import { copyText } from "../lib/profile";
import { APP_VERSION } from "../lib/updater";

export type CopyPhase = "idle" | "busy" | "copied" | "failed";

const LABELS: Record<CopyPhase, string> = {
  idle: "Copy diagnostics",
  busy: "Copying",
  copied: "Copied",
  failed: "Copy failed",
};

export const COPY_TITLE = "Copy a short report to paste into a GitHub issue";

/** The header has no room for the fallback report, so a refusal there points at the one that does. */
export const COPY_FAILED_TITLE = "Copy failed. Copy it from the waiting screen after the match";

/** How long "Copied" stays up before the link goes quiet again. */
const COPIED_MS = 2000;

/**
 * Without a backend there is no report, but there is still a user with a broken setup: copy
 * what the frontend knows so a plain-browser dev run, or a command that went missing, still
 * puts something pasteable on the clipboard.
 */
const unavailable = (cause: unknown): string =>
  `Valorant Lightweight Tracker diagnostics\napp: v${APP_VERSION}\ndiagnostics unavailable: ${String(cause)}`;

export interface CopyDiagnostics {
  phase: CopyPhase;
  /** The button's text for the current phase. */
  label: string;
  /** The report the clipboard refused, for a manual select-and-copy. Null until it does. */
  fallback: string | null;
  copy: (ui: UiFacts) => Promise<void>;
}

/**
 * The "Copy diagnostics" control's whole behaviour, shared by the waiting screen and the
 * header so both report the same states. A failed copy keeps its text around until a later
 * one succeeds — the fallback is the only way out for a webview with no clipboard.
 */
export function useCopyDiagnostics(): CopyDiagnostics {
  const [phase, setPhase] = useState<CopyPhase>("idle");
  const [fallback, setFallback] = useState<string | null>(null);

  useEffect(() => {
    if (phase !== "copied") return;
    const timer = setTimeout(() => setPhase("idle"), COPIED_MS);
    return () => clearTimeout(timer);
  }, [phase]);

  async function copy(ui: UiFacts): Promise<void> {
    setPhase("busy");
    const report = await getDiagnostics(ui).catch(unavailable);
    if (await copyText(report)) {
      setFallback(null);
      setPhase("copied");
    } else {
      setFallback(report);
      setPhase("failed");
    }
  }

  return { phase, label: LABELS[phase], fallback, copy };
}
