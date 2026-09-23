import {
  cloneElement,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type FocusEvent,
  type MouseEvent,
  type ReactElement,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

/** Hover time before a tooltip opens. Leaving closes it at once. */
const SHOW_DELAY_MS = 200;
/** Gap between trigger and bubble. */
const OFFSET = 6;
/** Closest the bubble may come to a window edge. */
const MARGIN = 8;

interface TriggerProps {
  onMouseEnter?: (event: MouseEvent<HTMLElement>) => void;
  onMouseLeave?: (event: MouseEvent<HTMLElement>) => void;
  onFocus?: (event: FocusEvent<HTMLElement>) => void;
  onBlur?: (event: FocusEvent<HTMLElement>) => void;
  "aria-describedby"?: string;
}

/** Above the trigger when it fits, else below; then shifted sideways to stay in the window. */
function place(bubble: HTMLElement, anchor: HTMLElement) {
  const a = anchor.getBoundingClientRect();
  const { width, height } = bubble.getBoundingClientRect();
  const vw = document.documentElement.clientWidth;
  const vh = document.documentElement.clientHeight;

  const above = a.top - OFFSET - height;
  const below = a.bottom + OFFSET;
  const fitsAbove = above >= MARGIN;
  const fitsBelow = below + height <= vh - MARGIN;
  const top = fitsAbove || (!fitsBelow && a.top > vh - a.bottom) ? above : below;
  const left = a.left + a.width / 2 - width / 2;

  bubble.style.top = `${Math.max(MARGIN, Math.min(top, vh - MARGIN - height))}px`;
  bubble.style.left = `${Math.max(MARGIN, Math.min(left, vw - MARGIN - width))}px`;
  bubble.style.visibility = "visible";
}

/**
 * Hover and keyboard-focus tooltip. Its props are merged onto the single child, so the trigger
 * keeps its own place in grid and flex layouts. The bubble is portalled to `<body>` with fixed
 * positioning, so no overflow container clips it and nothing around it moves. Plain-string
 * content keeps its line breaks. Empty `content` renders the child untouched.
 */
export function Tooltip({
  content,
  children,
}: {
  content: ReactNode;
  children: ReactElement<TriggerProps>;
}) {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const bubble = useRef<HTMLDivElement>(null);
  const id = useId();

  const cancel = () => window.clearTimeout(timer.current);
  const hide = () => {
    cancel();
    setAnchor(null);
  };

  useEffect(() => cancel, []);

  // Every render while open: the content, and with it the bubble's size, can change under it.
  useLayoutEffect(() => {
    if (!anchor || !bubble.current) return;
    if (anchor.isConnected) place(bubble.current, anchor);
    else setAnchor(null);
  });

  useEffect(() => {
    if (!anchor) return;
    const onKey = (event: KeyboardEvent) => event.key === "Escape" && hide();
    window.addEventListener("scroll", hide, true);
    window.addEventListener("resize", hide);
    window.addEventListener("pointerdown", hide, true);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("scroll", hide, true);
      window.removeEventListener("resize", hide);
      window.removeEventListener("pointerdown", hide, true);
      window.removeEventListener("keydown", onKey);
    };
  }, [anchor]);

  if (content == null || content === false || content === "") return children;

  const own = children.props;
  const trigger = cloneElement(children, {
    onMouseEnter: (event: MouseEvent<HTMLElement>) => {
      own.onMouseEnter?.(event);
      const target = event.currentTarget;
      cancel();
      timer.current = window.setTimeout(() => setAnchor(target), SHOW_DELAY_MS);
    },
    onMouseLeave: (event: MouseEvent<HTMLElement>) => {
      own.onMouseLeave?.(event);
      hide();
    },
    // Keyboard focus only: a click focuses too, and the pointer already has the hover path.
    onFocus: (event: FocusEvent<HTMLElement>) => {
      own.onFocus?.(event);
      if (event.target.matches(":focus-visible")) {
        cancel();
        setAnchor(event.currentTarget);
      }
    },
    onBlur: (event: FocusEvent<HTMLElement>) => {
      own.onBlur?.(event);
      hide();
    },
    "aria-describedby": anchor ? id : own["aria-describedby"],
  });

  return (
    <>
      {trigger}
      {anchor &&
        createPortal(
          <div
            ref={bubble}
            id={id}
            role="tooltip"
            className="pointer-events-none invisible fixed top-0 left-0 z-50 max-w-64 rounded-md border border-white/10 bg-edge px-2 py-1 text-[11px] leading-snug whitespace-pre-line text-neutral-200 shadow-lg shadow-black/60"
          >
            {content}
          </div>,
          document.body,
        )}
    </>
  );
}
