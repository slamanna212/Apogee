import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface MpvPropertyChangeEvent {
  event: string;
  id?: number;
  name?: string;
  data?: unknown;
}

export function loadUrl(url: string): Promise<void> {
  return invoke('mpv_load', { url });
}

export function setPause(paused: boolean): Promise<void> {
  return invoke('mpv_set_pause', { paused });
}

export function setVolume(volume: number): Promise<void> {
  return invoke('mpv_set_volume', { volume });
}

export function onMpvEvent(callback: (event: MpvPropertyChangeEvent) => void): Promise<UnlistenFn> {
  return listen<MpvPropertyChangeEvent>('mpv-event', (e) => callback(e.payload));
}
