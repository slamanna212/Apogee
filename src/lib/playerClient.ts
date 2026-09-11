import type { AudioBufferSettings } from './audioBuffer';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

// Typed facade over the Rust-owned Symphonia/CPAL playback engine. This is
// the only place the frontend should invoke `player_*` commands or listen
// for `player-snapshot` events - see src/stores/playerStore.ts, which treats
// everything here as the source of truth (a projection of Rust state, not a
// second retry/state machine).

export type PlaybackState = 'stopped' | 'connecting' | 'buffering' | 'playing' | 'recovering' | 'failed';

export type BufferingReason = 'connecting' | 'fillingBuffer' | 'underrun' | 'reconnecting' | null;

/** What Rust reports after each accepted play/stop and on every state change.
 *  `revision` is monotonically increasing across the whole player lifetime -
 *  never assume a snapshot with a lower or equal revision than one already
 *  applied is newer, even if it arrives later (see initSnapshotListener). */
export interface Snapshot {
  generation: number;
  revision: number;
  stationId: string | null;
  state: PlaybackState;
  bufferingReason: BufferingReason;
  attempt: number;
  bitrateKbps: number | null;
  sampleRate: number | null;
  device: string | null;
  error: string | null;
}

/** Enough for Rust to build both provider stream URL candidates and alternate
 *  between them itself - the frontend no longer constructs stream URLs. */
export interface StationRequest {
  baseUrl: string;
  username: string;
  password: string;
  streamId: number;
  stationId: string;
}

export interface DeviceDescriptor {
  id: string;
  name: string;
  backend: string;
  isDefault: boolean;
}

export function play(request: StationRequest): Promise<Snapshot> {
  return invoke('player_play', { request });
}

export function stop(): Promise<Snapshot> {
  return invoke('player_stop');
}

export function setVolume(volume: number): Promise<void> {
  return invoke('player_set_volume', { volume });
}

export function setMuted(muted: boolean): Promise<void> {
  return invoke('player_set_muted', { muted });
}

export function setEqualizer(enabled: boolean, gains: readonly number[]): Promise<void> {
  return invoke('player_set_equalizer', { enabled, gains });
}

export function listDevices(): Promise<DeviceDescriptor[]> {
  return invoke('player_list_devices');
}

export function setDevice(deviceId: string | null): Promise<void> {
  return invoke('player_set_device', { deviceId });
}

/** Migrates a legacy MPV device identifier to the new device model. Returns a
 *  human-readable explanation when the saved device could not be honoured
 *  (e.g. it no longer exists / has no CPAL equivalent), or `null` when
 *  nothing needs surfacing. Does not itself return a new device id - the
 *  effective device falls back to system default when this returns non-null. */
/** Outcome of migrating a device selection saved under the old MPV shape. */
export interface DeviceMigration {
  /**
   * The CPAL device id to persist, or null to follow the system default.
   * This must be written back to settings: the engine has already adopted it,
   * so persisting null instead would silently undo the migration on next launch.
   */
  deviceId: string | null;
  /** Why the stored selection could not be honoured, when it could not. */
  notice: string | null;
}

export function migrateDevice(stored: string | null): Promise<DeviceMigration> {
  return invoke('player_migrate_device', { stored });
}

/**
 * Enables or disables the spectrum visualizer. While disabled the engine performs
 * no FFT work at all, so a hidden visualizer is free.
 */
export function setVisualizer(enabled: boolean): Promise<void> {
  return invoke('player_set_visualizer', { enabled });
}

export function getSnapshot(): Promise<Snapshot> {
  return invoke('player_get_snapshot');
}

export function onSnapshot(callback: (snapshot: Snapshot) => void): Promise<UnlistenFn> {
  return listen<Snapshot>('player-snapshot', (e) => callback(e.payload));
}

/** Applies to the next playback session; does not interrupt current audio. */
export function setBuffering(buffering: AudioBufferSettings): Promise<void> {
  return invoke('player_set_buffering', { buffering });
}
