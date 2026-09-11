import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { debug as logDebug } from '@tauri-apps/plugin-log';
import { spectrumDisplayLevels } from '../lib/spectrumDisplay';
import { useSettingsStore } from '../stores/settingsStore';

const BASELINE = 0.08;
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
 * Measured differences are expanded for readability at this small size. While
 * waiting for levels during playback, a breathing animation fills the gap.
 * Stopped playback parks the bars at their baseline.
 */
export function Waveform({ active, bands = 8, size = 'md' }: WaveformProps) {
  const barRefs = useRef<(HTMLDivElement | null)[]>([]);
  const realLevelsRef = useRef<number[] | null>(null);
  const lastRealAtRef = useRef(0);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    listen<number[]>('waveform-levels', (e) => {
      realLevelsRef.current = e.payload;
      lastRealAtRef.current = performance.now();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
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
    let previous = start;
    let lastDiagnostic = start;
    const displayed = Array<number>(bands).fill(0);

    function tick(now: number) {
      const t = (now - start) / 1000;
      const elapsed = Math.min((now - previous) / 1000, 0.1);
      previous = now;
      const hasFreshLevels = now - lastRealAtRef.current < REAL_LEVELS_STALE_MS;
      const realLevels = hasFreshLevels ? realLevelsRef.current : null;
      const targets = realLevels && realLevels.length > 0
        ? spectrumDisplayLevels(realLevels, bands)
        : null;

      barRefs.current.forEach((el, i) => {
        if (!el) return;
        let amplitude = 0;
        if (targets) {
          amplitude = targets[i];
        } else {
          // No real capture backend available yet on this platform/session.
          const phase = i * 0.7;
          const speed = 1 + (i % 3) * 0.25;
          amplitude = 0.3 + 0.25 * ((Math.sin(t * speed * 4 + phase) + 1) / 2);
        }
        // A quick rise preserves transients. Time-based easing keeps movement
        // consistent across refresh rates without a continually retargeted CSS transition.
        const tau = amplitude > displayed[i] ? 0.025 : 0.12;
        displayed[i] += (amplitude - displayed[i]) * (1 - Math.exp(-elapsed / tau));
        el.style.transform = `scaleY(${BASELINE + displayed[i] * (1 - BASELINE)})`;
      });

      if (now - lastDiagnostic >= 5000 && useSettingsStore.getState().settings.verboseLogging) {
        lastDiagnostic = now;
        // Read geometry only during diagnostics, after all bar writes. This
        // distinguishes saturated input from a host webview rendering problem.
        void logDebug(`spectrum display: ${JSON.stringify({
          size, bands, fresh: hasFreshLevels, levels: realLevels, targets,
          bars: barRefs.current.map((el) => el ? {
            transform: el.style.transform,
            computedTransform: getComputedStyle(el).transform,
            layoutHeight: el.offsetHeight,
            renderedHeight: el.getBoundingClientRect().height,
          } : null),
        })}`).catch(() => {});
      }

      raf = requestAnimationFrame(tick);
    }
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [active, bands, size]);

  const barWidth = size === 'sm' ? 2.5 : 3;
  const gap = size === 'sm' ? 2 : 3;
  const height = size === 'sm' ? 18 : 28;
  // Leave room around the transformed bars so edge pixels do not sit on the
  // visualizer's raster boundary in WebView2, including at fractional DPI.
  const edgeInset = 2;

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap,
        height,
        width: barWidth * bands + gap * (bands - 1) + edgeInset * 2,
        paddingInline: edgeInset,
        boxSizing: 'border-box',
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
            flex: 'none',
            height: '100%',
            borderRadius: barWidth,
            background: 'var(--app-accent2)',
            transformOrigin: 'center',
            transform: `scaleY(${BASELINE})`,
          }}
        />
      ))}
    </div>
  );
}
