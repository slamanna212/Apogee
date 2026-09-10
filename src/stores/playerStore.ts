import { create } from 'zustand';
import { debug as logDebug, info as logInfo, warn as logWarn, error as logError } from '@tauri-apps/plugin-log';
import type { XtreamChannel } from '../types/xtream';
import type { PlayerState, PlayerStatus } from '../types/player';
import {
  onSnapshot,
  play as playerPlay,
  stop as playerStop,
  setVolume as playerSetVolume,
  setMuted as playerSetMuted,
  type Snapshot,
  type StationRequest,
} from '../lib/playerClient';
import type { XtreamCredentials } from '../lib/xtream';
import { onMediaControlEvent, setMediaPlayback, setMediaVolume } from '../lib/mediaSession';
import { setWaveformActive } from '../lib/waveform';
import { useSettingsStore } from './settingsStore';

// Plain console.* calls only reach a devtools console (invisible in a
// production build) - @tauri-apps/plugin-log's functions instead invoke the
// Rust log plugin's `log` command, so they land in the same exportable log
// file as the backend output. Use these for anything worth keeping.

// This store is a projection of Rust playback state, not a second retry/state
// machine: Rust owns connection attempts, extension fallback, and recovery
// (see SYMPHONIA_PLAYBACK_PLAN.md section 9). All we do here is subscribe to
// `player-snapshot` events, map them onto the existing PlayerStatus shape the
// UI already consumes, and forward user intent (play/stop/volume/mute) to the
// typed commands in src/lib/playerClient.ts.

// While status is 'playing', periodically log the current bitrate at debug
// level so a stretch of silence (no heartbeat) in the log is itself a signal
// something stalled, even when Rust never reports a hard error - the main gap
// for chasing intermittent playback issues that just go quiet.
const HEARTBEAT_INTERVAL_MS = 15_000;
let heartbeatTimer: ReturnType<typeof setInterval> | null = null;

function stopHeartbeat() {
  if (heartbeatTimer) {
    clearInterval(heartbeatTimer);
    heartbeatTimer = null;
  }
}

interface PlayerActions {
  initEventListener: () => void;
  selectChannel: (
    channel: XtreamChannel,
    creds: XtreamCredentials,
  ) => Promise<void>;
  play: () => Promise<void>;
  stop: () => Promise<void>;
  setVolume: (volume: number) => Promise<void>;
  toggleMute: () => Promise<void>;
}

type PlayerStore = PlayerState & PlayerActions;

let listening = false;
// Volume slider onChange fires on every pointer-move while dragging - debounce
// writing to settings.json so a drag doesn't hammer disk with one save per tick.
const VOLUME_PERSIST_DEBOUNCE_MS = 400;
let volumePersistTimer: ReturnType<typeof setTimeout> | null = null;

// Credentials for the channel currently selected, kept around so play() (used
// for OS-media-control resume after stop()) can reissue player_play without
// the caller passing creds again.
let activeCreds: XtreamCredentials | null = null;

// Guards against a stale snapshot overwriting newer state: an event can
// arrive before its originating invoke() resolves (e.g. player_play's return
// value racing the event stream), and snapshots can arrive out of order
// across generations. Only ever move forward. Starts at -1 so revision 0 -
// the very first snapshot a fresh backend could emit - is still accepted.
let lastAppliedRevision = -1;

function mapStatus(state: Snapshot['state']): { status: PlayerStatus; isBuffering: boolean } {
  switch (state) {
    case 'stopped':
      return { status: 'stopped', isBuffering: false };
    case 'connecting':
    case 'buffering':
    case 'recovering':
      return { status: 'loading', isBuffering: true };
    case 'playing':
      return { status: 'playing', isBuffering: false };
    case 'failed':
      return { status: 'error', isBuffering: false };
  }
}

export const usePlayerStore = create<PlayerStore>((set, get) => {
  function startHeartbeat(streamId: number) {
    stopHeartbeat();
    heartbeatTimer = setInterval(() => {
      const state = get();
      if (state.status !== 'playing' || state.currentChannel?.stream_id !== streamId) {
        stopHeartbeat();
        return;
      }
      logDebug(`heartbeat: channel ${state.currentChannel?.name ?? streamId} playing, bitrate=${state.bitrateKbps ?? 'unknown'}kbps`);
    }, HEARTBEAT_INTERVAL_MS);
  }

  // Applies a snapshot from Rust (either the return value of an invoke, or a
  // `player-snapshot` event) if - and only if - it is newer than the last one
  // applied. This is the single place backend state becomes frontend state.
  function applySnapshot(snapshot: Snapshot) {
    if (snapshot.revision <= lastAppliedRevision) {
      logWarn(`ignoring stale player snapshot (revision ${snapshot.revision} <= ${lastAppliedRevision})`);
      return;
    }
    lastAppliedRevision = snapshot.revision;

    const prevStatus = get().status;
    const { status, isBuffering } = mapStatus(snapshot.state);
    const errorMessage = snapshot.state === 'failed' ? (snapshot.error ?? 'Playback failed') : null;

    set({
      status,
      isBuffering,
      bitrateKbps: snapshot.bitrateKbps,
      errorMessage,
    });

    if (status === 'playing' && prevStatus !== 'playing') {
      const channel = get().currentChannel;
      logInfo(`playback started for channel ${channel?.name ?? channel?.stream_id ?? snapshot.stationId}`);
      setMediaPlayback(true);
      setWaveformActive(true);
      if (channel) startHeartbeat(channel.stream_id);
    } else if (status !== 'playing' && prevStatus === 'playing') {
      stopHeartbeat();
      setWaveformActive(false);
      setMediaPlayback(false);
    }

    if (status === 'error') {
      logError(`playback failed for station ${snapshot.stationId ?? get().currentChannel?.name}: ${errorMessage}`);
      setWaveformActive(false);
    }
  }

  function buildRequest(channel: XtreamChannel, creds: XtreamCredentials): StationRequest {
    return {
      baseUrl: creds.baseUrl,
      username: creds.username,
      password: creds.password,
      streamId: channel.stream_id,
      stationId: String(channel.stream_id),
    };
  }

  async function requestPlay(channel: XtreamChannel, creds: XtreamCredentials) {
    logInfo(`connecting to channel ${channel.name}`);
    try {
      const snapshot = await playerPlay(buildRequest(channel, creds));
      applySnapshot(snapshot);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      logError(`connect failed for channel ${channel.name}: ${message}`);
      set({ status: 'error', errorMessage: message, isBuffering: false });
      setWaveformActive(false);
      throw err;
    }
  }

  return {
    status: 'idle',
    currentChannel: null,
    volume: 80,
    muted: false,
    bitrateKbps: null,
    errorMessage: null,
    isBuffering: false,

    initEventListener() {
      if (listening) return;
      listening = true;

      // Register the snapshot listener before any command is ever issued -
      // an event can arrive before its own invoke() resolves.
      onSnapshot((snapshot) => {
        if (useSettingsStore.getState().settings.verboseLogging) {
          logDebug(`player snapshot: ${JSON.stringify(snapshot)}`);
        }
        applySnapshot(snapshot);
      });

      onMediaControlEvent((kind, value) => {
        if (!get().currentChannel) return;
        if (kind === 'play') {
          get().play();
        } else if (kind === 'volume' && value != null) {
          get().setVolume(Math.round(value * 100));
        } else {
          // 'pause' and 'toggle' both mean "stop" - live radio has no pause.
          get().stop();
        }
      });
    },

    async selectChannel(channel, creds) {
      activeCreds = creds;
      set({ currentChannel: channel });
      await requestPlay(channel, creds);
    },

    async play() {
      const channel = get().currentChannel;
      if (!channel || !activeCreds) return;
      await requestPlay(channel, activeCreds);
    },

    async stop() {
      logInfo(`stopping channel ${get().currentChannel?.name ?? get().currentChannel?.stream_id}`);
      stopHeartbeat();
      const snapshot = await playerStop();
      applySnapshot(snapshot);
      setWaveformActive(false);
      await setMediaPlayback(false);
    },

    async setVolume(volume) {
      // Dragging the slider while muted would otherwise look like it's doing
      // nothing (audio stays silent) - unmute so it takes audible effect.
      if (get().muted) {
        set({ muted: false });
        await playerSetMuted(false);
      }
      set({ volume });
      await playerSetVolume(volume);
      // Echo back to the OS media widget unconditionally (not just for
      // MPRIS-originated changes) - the MPRIS spec requires this after any
      // volume change or the widget's own slider drifts out of sync.
      await setMediaVolume(volume / 100);
      if (volumePersistTimer) clearTimeout(volumePersistTimer);
      volumePersistTimer = setTimeout(() => {
        volumePersistTimer = null;
        useSettingsStore.getState().update({ volume });
      }, VOLUME_PERSIST_DEBOUNCE_MS);
    },

    async toggleMute() {
      const next = !get().muted;
      set({ muted: next });
      await playerSetMuted(next);
    },
  };
});
