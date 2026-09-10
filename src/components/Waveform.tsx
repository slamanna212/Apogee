import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';

const BASELINE = 0.12;
const REAL_LEVELS_STALE_MS = 1000;

interface WaveformProps {
  /** true while playback status is 'playing' */
  active: boolean;
  bands?: number;
  size?: 'md' | 'sm';
}

/**
 * Real per-band levels come from the Rust "waveform-levels" event. The playback
 * engine taps its own decoded PCM after EQ and volume and runs an FFT over it
 * (src-tauri/playback-core/src/analysis.rs), so the bars show what Apogee is
 * sending to the output device rather than everything the machine is playing.
 *
 * Historical: this used to capture system audio through a loopback device,
 * because mpv's af-metadata mechanism was tested and proven unable to expose
 * more than one overall level (ffmpeg's amix/merge filters drop per-branch
 * metadata). That capture path is gone, along with its permission requirements.
 *
 * While nothing is playing, bars fall back to a synthetic idle-breathing
 * animation so they never look broken - but whenever real levels are flowing,
 * that's exactly what's rendered.
 */
export function Waveform({ active, bands = 8, size = 'md' }: WaveformProps) {
  const barRefs = useRef<(HTMLDivElement | null)[]>([]);
  const realLevelsRef = useRef<number[] | null>(null);
  const lastRealAtRef = useRef(0);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<number[]>('waveform-levels', (e) => {
      realLevelsRef.current = e.payload;
      lastRealAtRef.current = performance.now();
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    // While stopped there's nothing to animate - park every bar at BASELINE
    // once and don't schedule a permanent 60fps rAF loop over silence. The
    // effect re-runs when `active` flips, restarting the loop on playback.
    if (!active) {
      barRefs.current.forEach((el) => {
        if (el) el.style.transform = `scaleY(${BASELINE})`;
      });
      return;
    }

    let raf: number;
    const start = performance.now();

    function tick(now: number) {
      const t = (now - start) / 1000;
      const hasFreshLevels = now - lastRealAtRef.current < REAL_LEVELS_STALE_MS;
      const realLevels = hasFreshLevels ? realLevelsRef.current : null;

      barRefs.current.forEach((el, i) => {
        if (!el) return;
        let amplitude = 0;
        if (realLevels && realLevels.length > 0) {
          const bandIndex = Math.min(realLevels.length - 1, Math.floor((i / bands) * realLevels.length));
          amplitude = realLevels[bandIndex] ?? 0;
        } else {
          // No real capture backend available yet on this platform/session.
          const phase = i * 0.7;
          const speed = 1 + (i % 3) * 0.25;
          amplitude = 0.3 + 0.25 * ((Math.sin(t * speed * 4 + phase) + 1) / 2);
        }
        el.style.transform = `scaleY(${BASELINE + amplitude * 0.88})`;
      });

      raf = requestAnimationFrame(tick);
    }
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [active, bands]);

  const barWidth = size === 'sm' ? 2.5 : 3;
  const gap = size === 'sm' ? 2 : 3;
  const height = size === 'sm' ? 18 : 28;

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap,
        height,
        width: barWidth * bands + gap * (bands - 1),
        flex: 'none',
      }}
    >
      {Array.from({ length: bands }).map((_, i) => (
        <div
          key={i}
          ref={(el) => {
            barRefs.current[i] = el;
          }}
          style={{
            width: barWidth,
            height: '100%',
            borderRadius: barWidth,
            background: 'var(--app-accent2)',
            transformOrigin: 'center',
            transition: 'transform 300ms ease-out',
          }}
        />
      ))}
    </div>
  );
}
