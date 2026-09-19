import { useEffect, useRef, useState, type CSSProperties } from 'react';

const SCROLL_PX_PER_SECOND = 40;
const END_HOLD_MS = 1500;
const RETURN_MS = 400;

interface MarqueeTextProps {
  text: string;
  /** Font, color etc. for the text itself; layout is owned here. */
  textStyle: CSSProperties;
  /** Seconds the text rests at its start between one scroll and the next. */
  periodSeconds: number;
  /** When false, an overflowing title is ellipsised instead of scrolled. */
  enabled: boolean;
  size?: 'md' | 'sm';
}

function easeInOutQuad(t: number): number {
  return t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2;
}

function edgeStyle(side: 'left' | 'right', width: number, blur: number): CSSProperties {
  const mask = `linear-gradient(${side === 'right' ? '270deg' : '90deg'}, #000, transparent)`;
  return {
    position: 'absolute',
    top: 0,
    bottom: 0,
    [side]: 0,
    width,
    pointerEvents: 'none',
    opacity: 0,
    backdropFilter: `blur(${blur}px)`,
    WebkitBackdropFilter: `blur(${blur}px)`,
    maskImage: mask,
    WebkitMaskImage: mask,
  };
}

/**
 * A single-line title that, when it doesn't fit, holds still and then scrolls once every
 * `periodSeconds` instead of being cut off with an ellipsis. Titles that fit are plain
 * text with no animation. Scrolling is driven by one rAF loop writing `transform`
 * directly (the Waveform pattern), and only while a scroll is actually in progress; the
 * hold between scrolls is a plain timeout.
 */
export function MarqueeText({ text, textStyle, periodSeconds, enabled, size = 'md' }: MarqueeTextProps) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const innerRef = useRef<HTMLDivElement>(null);
  const leftRef = useRef<HTMLDivElement>(null);
  const rightRef = useRef<HTMLDivElement>(null);
  const [overflow, setOverflow] = useState(0);
  const [reducedMotion, setReducedMotion] = useState(
    () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches,
  );

  useEffect(() => {
    if (typeof matchMedia !== 'function') return;
    const query = matchMedia('(prefers-reduced-motion: reduce)');
    const onChange = () => setReducedMotion(query.matches);
    query.addEventListener('change', onChange);
    return () => query.removeEventListener('change', onChange);
  }, []);

  // Measure on mount, on text change, when fonts finish loading, and whenever either box
  // resizes. Webfont metrics land late, so a single mount-time measurement can read 0 and
  // freeze the lane permanently.
  useEffect(() => {
    const wrap = wrapRef.current;
    const inner = innerRef.current;
    if (!wrap || !inner || !enabled) {
      setOverflow(0);
      return;
    }
    let disposed = false;
    const measure = () => {
      if (disposed) return;
      setOverflow(Math.max(0, Math.ceil(inner.scrollWidth - wrap.clientWidth)));
    };
    measure();
    void document.fonts?.ready.then(measure);
    const observer = new ResizeObserver(measure);
    observer.observe(wrap);
    observer.observe(inner);
    return () => {
      disposed = true;
      observer.disconnect();
    };
  }, [text, enabled]);

  useEffect(() => {
    const inner = innerRef.current;
    const left = leftRef.current;
    const right = rightRef.current;
    if (!inner || !left || !right) return;

    const place = (x: number) => {
      inner.style.transform = `translateX(${x}px)`;
      left.style.opacity = x < -4 ? '1' : '0';
      right.style.opacity = overflow > 0 && x > -overflow + 1 ? '1' : '0';
    };

    place(0);
    if (overflow <= 0 || reducedMotion) return;

    const travelMs = (overflow / SCROLL_PX_PER_SECOND) * 1000;
    const activeMs = travelMs + END_HOLD_MS + RETURN_MS;
    // The full period is time spent still at the start, not start-to-start: counting the
    // scroll itself against it left long titles only a few seconds of rest between scrolls.
    const holdMs = periodSeconds * 1000;

    let raf = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let scrollStart = 0;

    const frame = (now: number) => {
      const u = now - scrollStart;
      if (u >= activeMs) {
        place(0);
        timer = setTimeout(beginScroll, holdMs);
        return;
      }
      let x: number;
      if (u < travelMs) x = -overflow * easeInOutQuad(u / travelMs);
      else if (u < travelMs + END_HOLD_MS) x = -overflow;
      else x = -overflow * (1 - easeInOutQuad((u - travelMs - END_HOLD_MS) / RETURN_MS));
      place(x);
      raf = requestAnimationFrame(frame);
    };

    function beginScroll() {
      if (document.visibilityState === 'hidden') return;
      scrollStart = performance.now();
      raf = requestAnimationFrame(frame);
    }

    const halt = () => {
      cancelAnimationFrame(raf);
      if (timer) clearTimeout(timer);
    };

    // Hidden windows (e.g. minimised mini player) don't scroll; a fresh full hold starts
    // when the window comes back.
    const onVisibility = () => {
      halt();
      place(0);
      if (document.visibilityState !== 'hidden') timer = setTimeout(beginScroll, holdMs);
    };
    document.addEventListener('visibilitychange', onVisibility);

    timer = setTimeout(beginScroll, holdMs);
    return () => {
      halt();
      document.removeEventListener('visibilitychange', onVisibility);
    };
  }, [overflow, periodSeconds, reducedMotion, text]);

  const edgeWidth = size === 'sm' ? 14 : 20;
  const blur = size === 'sm' ? 1 : 1.5;

  return (
    <div data-tauri-drag-region ref={wrapRef} style={{ position: 'relative', overflow: 'hidden', whiteSpace: 'nowrap', minWidth: 0 }}>
      <div
        data-tauri-drag-region
        ref={innerRef}
        style={{
          ...textStyle,
          display: enabled ? 'inline-block' : 'block',
          whiteSpace: 'nowrap',
          overflow: enabled ? 'visible' : 'hidden',
          textOverflow: enabled ? 'clip' : 'ellipsis',
          willChange: overflow > 0 && !reducedMotion ? 'transform' : undefined,
        }}
      >
        {text}
      </div>
      <div ref={leftRef} style={edgeStyle('left', size === 'sm' ? 12 : 16, blur)} />
      <div ref={rightRef} style={edgeStyle('right', edgeWidth, blur)} />
    </div>
  );
}
