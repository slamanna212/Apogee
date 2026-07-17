import { invoke } from '@tauri-apps/api/core';

// Points the audio-spectrum visualizer at a specific output device so it
// captures the same audio mpv is playing. Pass `null`/`null` for the system
// default. `name` is mpv's `audio-device-list` name; `description` is the
// friendly label (used to match the device on Windows).
export function setWaveformDevice(
  name: string | null,
  description: string | null,
): Promise<void> {
  return invoke('waveform_set_device', { name, description });
}
