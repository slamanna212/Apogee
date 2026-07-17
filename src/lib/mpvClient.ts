import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface MpvPropertyChangeEvent {
  event?: string;
  id?: number;
  name?: string;
  data?: unknown;
  request_id?: number;
  // Present on 'end-file' events - mpv's JSON IPC reports one of "eof",
  // "stop", "quit", "error", "redirect".
  reason?: string;
}

// Must match GET_PROPERTY_REQUEST_ID in src-tauri/src/mpv.rs.
export const GET_PROPERTY_REQUEST_ID = 777;

export function loadUrl(url: string): Promise<void> {
  return invoke('mpv_load', { url });
}

export function stopPlayback(): Promise<void> {
  return invoke('mpv_stop');
}

export function setVolume(volume: number): Promise<void> {
  return invoke('mpv_set_volume', { volume });
}

export function setProperty(name: string, value: unknown): Promise<void> {
  return invoke('mpv_set_property', { name, value });
}

export function setMute(muted: boolean): Promise<void> {
  return setProperty('mute', muted);
}

export function getProperty(name: string): Promise<void> {
  return invoke('mpv_get_property', { name });
}

export interface AudioDevice {
  name: string;
  description: string;
}

// Enumerates mpv's available output devices. Spawns mpv idle on demand if it
// isn't already running, so it can be called before any playback has started.
export function listAudioDevices(): Promise<AudioDevice[]> {
  return invoke('mpv_list_audio_devices');
}

export function getStderrTail(): Promise<string> {
  return invoke('mpv_get_stderr_tail');
}

export function onMpvEvent(callback: (event: MpvPropertyChangeEvent) => void): Promise<UnlistenFn> {
  return listen<MpvPropertyChangeEvent>('mpv-event', (e) => callback(e.payload));
}
