import { useEffect, useState } from "react";
import { useUpdateState } from "../hooks/useUpdateState";
import { APP_VERSION, runUpdateCheck, type UpdateCheck } from "../lib/updater";

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

function resultText(check: UpdateCheck): string {
  switch (check.state) {
    case "upToDate":
      return "Up to date";
    case "available":
      return `Update: v${check.version}`;
    case "error":
      return "Check failed";
  }
}

/** The version, doubling as the update control the future auto-updater will report through. */
function VersionLine() {
  const { checking, result } = useUpdateState();
  const [showResult, setShowResult] = useState(false);

  useEffect(() => {
    if (!result) return;
    setShowResult(true);
    const timer = setTimeout(() => setShowResult(false), RESULT_MS);
    return () => clearTimeout(timer);
  }, [result]);

  const shown = showResult ? result : null;
  const tone = shown?.state === "available" ? "text-accent/70" : "text-neutral-600";

  return (
    <button
      type="button"
      onClick={runUpdateCheck}
      title="Check for updates"
      className={`mt-6 text-[10px] tabular-nums ${tone} transition-colors hover:text-neutral-300 ${
        checking ? "animate-pulse" : ""
      }`}
    >
      {shown ? resultText(shown) : `v${APP_VERSION}`}
    </button>
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
}: {
  title: string;
  subtitle?: string | null;
  tone: Tone;
  /** Optional side door out of the waiting screen — today, the held last-match table. */
  action?: StatusAction | null;
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
          <p className="mt-2 text-[12px] leading-relaxed text-neutral-500">{subtitle}</p>
        )}
        {action && (
          <button
            type="button"
            onClick={action.onClick}
            className="mt-5 rounded-full border border-edge px-3.5 py-1.5 text-[11px] text-neutral-400 transition-colors hover:border-neutral-600 hover:text-neutral-200"
          >
            {action.label}
          </button>
        )}
        <VersionLine />
      </div>
    </div>
  );
}
