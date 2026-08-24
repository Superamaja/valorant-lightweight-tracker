import { useState, type ReactNode } from "react";

interface ImgProps {
  src: string | null;
  alt: string;
  /** Tooltip; defaults to `alt` — this is how the spec's "names are tooltips" works. */
  title?: string;
  className?: string;
  /** Rendered when there is no URL or the image fails to load. */
  fallback?: ReactNode;
}

/** An image that degrades quietly: null URLs and 404s fall back instead of showing a break. */
export function Img({ src, alt, title, className, fallback = null }: ImgProps) {
  const [failedSrc, setFailedSrc] = useState<string | null>(null);

  if (!src || failedSrc === src) return <>{fallback}</>;

  return (
    <img
      src={src}
      alt={alt}
      title={title ?? alt}
      className={className}
      draggable={false}
      onError={() => setFailedSrc(src)}
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

export const CheckIcon = () => (
  <svg viewBox="0 0 16 16" className={ICON} aria-hidden="true" {...STROKE}>
    <path d="m3.25 8.5 3.25 3.25L12.75 4.5" />
  </svg>
);
