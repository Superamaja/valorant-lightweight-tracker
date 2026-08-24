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

/** Every non-match screen. These are the ones the user sees most, so they get the room. */
export function StatusScreen({
  title,
  subtitle,
  tone,
}: {
  title: string;
  subtitle?: string | null;
  tone: Tone;
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

      <div className="max-w-sm">
        <h2 className="text-[15px] font-medium tracking-wide text-neutral-200">{title}</h2>
        {subtitle && (
          <p className="mt-2 text-[12px] leading-relaxed text-neutral-500">{subtitle}</p>
        )}
      </div>
    </div>
  );
}
