import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface MpvPropertyChangeEvent {
  event?: string;
  id?: number;
  name?: string;
  data?: unknown;
  request_id?: number;
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

export function getProperty(name: string): Promise<void> {
  return invoke('mpv_get_property', { name });
}

export function onMpvEvent(callback: (event: MpvPropertyChangeEvent) => void): Promise<UnlistenFn> {
  return listen<MpvPropertyChangeEvent>('mpv-event', (e) => callback(e.payload));
}
