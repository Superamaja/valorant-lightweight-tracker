import { useEffect, useState, type ReactNode } from "react";

interface ImgProps {
  src: string | null;
  alt: string;
  /** Tooltip; defaults to `alt` — this is how the spec's "names are tooltips" works. */
  title?: string;
  className?: string;
  /** Rendered when there is no URL or the image fails to load. */
  fallback?: ReactNode;
}

/** Delay before each retry of a failed URL; its length is also the retry budget. */
const RETRY_DELAYS_MS = [500, 2500];

interface Failure {
  src: string;
  /** How many loads of `src` have failed so far. */
  count: number;
  /** True while the retry delay is running; the fallback stands in meanwhile. */
  waiting: boolean;
}

/** An image that degrades quietly: null URLs and 404s fall back instead of showing a break. */
export function Img({ src, alt, title, className, fallback = null }: ImgProps) {
  const [failure, setFailure] = useState<Failure | null>(null);
  // A changed URL starts over: the old failure no longer describes what is rendered.
  const current = failure?.src === src ? failure : null;
  const count = current?.count ?? 0;
  const waiting = current?.waiting ?? false;

  useEffect(() => {
    if (!src || !waiting) return;
    const timer = setTimeout(
      () => setFailure({ src, count, waiting: false }),
      RETRY_DELAYS_MS[count - 1],
    );
    return () => clearTimeout(timer);
  }, [src, count, waiting]);

  if (!src || waiting || count > RETRY_DELAYS_MS.length) return <>{fallback}</>;

  return (
    <img
      // Remounting is what makes the browser request the same URL again.
      key={count}
      src={src}
      alt={alt}
      title={title ?? alt}
      className={className}
      draggable={false}
      onError={() =>
        setFailure({ src, count: count + 1, waiting: count < RETRY_DELAYS_MS.length })
      }
    />
  );
}

/** Placeholder for a stat still in flight. The caller sets size and radius. */
export const Skeleton = ({ className = "" }: { className?: string }) => (
  <span className={`block animate-pulse bg-white/10 ${className}`} />
);

const ICON = "h-3 w-3";
const STROKE = {
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.5,
  strokeLinecap: "round",
  strokeLinejoin: "round",
} as const;

export const CopyIcon = () => (
  <svg viewBox="0 0 16 16" className={ICON} aria-hidden="true" {...STROKE}>
    <rect x="5.75" y="5.75" width="8" height="8" rx="1.6" />
    <path d="M10.25 3.6A1.6 1.6 0 0 0 8.65 2H3.85A1.6 1.6 0 0 0 2.25 3.6v4.8a1.6 1.6 0 0 0 1.6 1.6" />
  </svg>
);

export const ArrowLeftIcon = () => (
  <svg viewBox="0 0 16 16" className={ICON} aria-hidden="true" {...STROKE}>
    <path d="M12.75 8H3.25m0 0 3.5-3.5M3.25 8l3.5 3.5" />
  </svg>
);

export const CheckIcon = () => (
  <svg viewBox="0 0 16 16" className={ICON} aria-hidden="true" {...STROKE}>
    <path d="m3.25 8.5 3.25 3.25L12.75 4.5" />
  </svg>
);

/** GitHub mark from Simple Icons (CC0 1.0), path copied verbatim. */
export const GitHubIcon = () => (
  <svg viewBox="0 0 24 24" className="h-2.5 w-2.5" fill="currentColor" aria-hidden="true">
    <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
  </svg>
);
