import { setVisualizer } from './playerClient';

// Gates the spectrum visualizer.
//
// This used to capture system audio through a loopback device so it could
// analyse whatever mpv was playing. The Symphonia/CPAL engine instead taps the
// PCM Apogee itself is about to hand the output device, post-EQ and post-volume,
// so the display reflects this app's output rather than everything the machine is
// playing - and no microphone or system-audio permission is involved.
//
// The backend emits the same `waveform-levels` event with the same 8-band
// payload, so `Waveform.tsx` is unchanged. Analysis is skipped entirely while
// this is off, so a hidden visualizer costs nothing.
export function setWaveformActive(active: boolean): Promise<void> {
  return setVisualizer(active);
}
