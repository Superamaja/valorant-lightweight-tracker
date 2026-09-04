import { useEffect, useState } from "react";
import { COPY_TITLE, useCopyDiagnostics } from "../hooks/useCopyDiagnostics";
import { useUpdateState } from "../hooks/useUpdateState";
import { openBugReport } from "../lib/profile";
import {
  APP_VERSION,
  runUpdateCheck,
  runUpdateInstall,
  type UpdateCheck,
} from "../lib/updater";
import { GitHubIcon } from "./primitives";

export type Tone = "idle" | "live" | "error";

const TONES: Record<Tone, { dot: string; ring: string; glow: string; pulsing: boolean }> = {
  // Nothing is connected — stay quiet.
  idle: { dot: "bg-neutral-600", ring: "border-neutral-700/70", glow: "", pulsing: true },
  // Connected and waiting — the one accent colour, alive.
  live: {
    dot: "bg-accent",
    ring: "border-accent/30",
    glow: "shadow-[0_0_20px_2px] shadow-accent/25",
    pulsing: true,
  },
  error: { dot: "bg-accent", ring: "border-accent/30", glow: "", pulsing: false },
};

/** How long a finished check's wording stays up before the line falls back to the version. */
const RESULT_MS = 4000;

const BUG_TITLE = "Open a bug report on GitHub with the version filled in";

function resultText(check: Exclude<UpdateCheck, { state: "available" }>): string {
  switch (check.state) {
    case "upToDate":
      return "Up to date";
    case "error":
      return "Check failed";
  }
}

/**
 * The offer itself, once a check has found a newer version: the loudest thing on the screen,
 * because the screen it sits on is one the user is already waiting through. Clicking installs
 * and restarts.
 */
function UpdateCallToAction() {
  const { result, installing, installError } = useUpdateState();

  if (result?.state !== "available") return null;

  const label = installing
    ? "Updating"
    : `${installError ? "Retry update to" : "Update to"} v${result.version}`;

  return (
    <button
      type="button"
      onClick={runUpdateInstall}
      disabled={installing}
      title={
        installing
          ? "Downloading the update"
          : (installError ?? "Install it and restart")
      }
      className={`mt-5 rounded-full bg-accent px-5 py-2.5 text-[13px] font-medium text-white shadow-[0_0_24px_-4px] shadow-accent/50 transition-opacity hover:opacity-90 ${
        installing ? "animate-pulse" : ""
      }`}
    >
      {label}
    </button>
  );
}

/** The version, doubling as the update control the auto-updater reports through. */
function VersionLine() {
  const { checking, installing, result, installError } = useUpdateState();
  const [showResult, setShowResult] = useState(false);

  useEffect(() => {
    if (!result) return;
    setShowResult(true);
    const timer = setTimeout(() => setShowResult(false), RESULT_MS);
    return () => clearTimeout(timer);
  }, [result]);

  // An available update has the call to action above; the line stays out of its way.
  const shown = showResult && result?.state !== "available" ? result : null;
  const busy = checking || installing;

  return (
    <button
      type="button"
      onClick={runUpdateCheck}
      disabled={busy}
      title={installError ?? "Check for updates"}
      className={`text-[10px] tabular-nums text-neutral-600 transition-colors hover:text-neutral-300 ${
        busy ? "animate-pulse" : ""
      }`}
    >
      {shown ? resultText(shown) : `v${APP_VERSION}`}
    </button>
  );
}

/** Nothing to read, everything to copy: the textarea exists to be select-all'd. */
const autoSelect = (field: HTMLTextAreaElement | null) => {
  field?.select();
};

/**
 * The quiet row under the subtitle: the version, beside it the way to hand a report to whoever
 * is being asked for help, and last the place to file it. When the clipboard refuses, the
 * report itself appears.
 */
function StatusFooter({ screen, heldTable }: { screen: string; heldTable: boolean }) {
  const { phase, label, fallback, copy } = useCopyDiagnostics();

  return (
    <div className="relative mt-6 flex flex-col items-center">
      <div className="flex items-center gap-2 text-[10px] text-neutral-600">
        <VersionLine />
        <span aria-hidden="true">·</span>
        <button
          type="button"
          onClick={() => void copy({ screen, heldTable })}
          disabled={phase === "busy"}
          title={COPY_TITLE}
          className={`text-[10px] transition-colors ${
            phase === "copied" ? "text-win" : "text-neutral-600 hover:text-neutral-300"
          } ${phase === "busy" ? "animate-pulse" : ""}`}
        >
          {label}
        </button>
        <span aria-hidden="true">·</span>
        <button
          type="button"
          onClick={() => void openBugReport()}
          title={BUG_TITLE}
          className="inline-flex items-center gap-1 text-[10px] text-neutral-600 transition-colors hover:text-neutral-300"
        >
          <GitHubIcon />
          Report a bug
        </button>
      </div>
      {fallback !== null && (
        // Out of flow, so the report appearing never moves the centred stack above it.
        <div className="absolute top-full left-1/2 mt-2 flex -translate-x-1/2 flex-col items-center">
          <textarea
            readOnly
            value={fallback}
            // A later report is a new field, so it gets selected the same way the first did.
            key={fallback}
            ref={autoSelect}
            className="max-h-40 w-96 resize-none rounded-sm border border-edge bg-white/[0.03] p-2 font-mono text-[10px] leading-snug text-neutral-400 field-sizing-content select-text selection:bg-white/15 selection:text-neutral-200 focus:outline-none"
          />
          <p className="mt-1 text-[10px] text-neutral-600">Select all and copy</p>
        </div>
      )}
    </div>
  );
}

export interface StatusAction {
  label: string;
  onClick: () => void;
}

/** Every non-match screen. These are the ones the user sees most, so they get the room. */
export function StatusScreen({
  title,
  subtitle,
  tone,
  action,
  heldTable = false,
}: {
  title: string;
  subtitle?: string | null;
  tone: Tone;
  /** Optional side door out of the waiting screen — today, the held last-match table. */
  action?: StatusAction | null;
  /** Whether a finished match's table is waiting behind this screen; for the report only. */
  heldTable?: boolean;
}) {
  const style = TONES[tone];

  return (
    <div className="flex h-full flex-col items-center justify-center gap-7 px-8 text-center">
      <div className="relative flex h-24 w-24 items-center justify-center">
        {style.pulsing && (
          <>
            <span
              className={`absolute inset-0 animate-pulse-ring rounded-full border ${style.ring}`}
            />
            <span
              className={`absolute inset-0 animate-pulse-ring rounded-full border ${style.ring} [animation-delay:1.4s]`}
            />
          </>
        )}
        <span className={`h-2.5 w-2.5 rounded-full ${style.dot} ${style.glow}`} />
      </div>

      <div className="flex max-w-sm flex-col items-center">
        <h2 className="text-[18px] font-medium tracking-wide text-neutral-200">{title}</h2>
        {subtitle && (
          <p className="mt-2 text-[13px] leading-relaxed text-neutral-500">{subtitle}</p>
        )}
        {action && (
          <button
            type="button"
            onClick={action.onClick}
            className="mt-5 rounded-full border border-edge px-3.5 py-1.5 text-[12px] text-neutral-400 transition-colors hover:border-neutral-600 hover:text-neutral-200"
          >
            {action.label}
          </button>
        )}
        <UpdateCallToAction />
        <StatusFooter screen={title} heldTable={heldTable} />
      </div>
    </div>
  );
}
