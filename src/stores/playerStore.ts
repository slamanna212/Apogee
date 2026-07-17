import { create } from 'zustand';
import { debug as logDebug, info as logInfo, warn as logWarn, error as logError } from '@tauri-apps/plugin-log';
import type { XtreamChannel } from '../types/xtream';
import type { PlayerState } from '../types/player';
import {
  GET_PROPERTY_REQUEST_ID,
  getProperty,
  getStderrTail,
  loadUrl,
  onMpvEvent,
  stopPlayback,
  setProperty as mpvSetProperty,
  setVolume as mpvSetVolume,
  setMute as mpvSetMute,
} from '../lib/mpvClient';
import { buildStreamUrl, type XtreamCredentials } from '../lib/xtream';
import { onMediaControlEvent, setMediaPlayback, setMediaVolume } from '../lib/mediaSession';
import { useSettingsStore } from './settingsStore';

// Plain console.* calls only reach a devtools console (invisible in a
// production build) - @tauri-apps/plugin-log's functions instead invoke the
// Rust log plugin's `log` command, so they land in the same exportable log
// file as the mpv/backend output. Use these for anything worth keeping.

// While status is 'playing', periodically log the current bitrate at debug
// level so a stretch of silence (no heartbeat) in the log is itself a signal
// something stalled, even when mpv never reports a hard error - the main gap
// for chasing intermittent Mac playback issues that just go quiet.
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
let fallbackTimer: ReturnType<typeof setInterval> | null = null;
let fallbackStartTimer: ReturnType<typeof setTimeout> | null = null;
// Volume slider onChange fires on every pointer-move while dragging - debounce
// writing to settings.json so a drag doesn't hammer disk with one save per tick.
const VOLUME_PERSIST_DEBOUNCE_MS = 400;
let volumePersistTimer: ReturnType<typeof setTimeout> | null = null;
// Last connected stream URL, kept around so `play()` can reconnect after a
// `stop()` without needing the channel to be reselected from the list.
let lastStreamUrl: string | null = null;
// Credentials for the channel currently being connected to, kept around so a
// retry (which may switch stream extension - see extensionForAttempt) can
// rebuild the URL without the caller needing to pass streamExtension around.
let activeCreds: XtreamCredentials | null = null;

// Some Xtream providers only spin the upstream channel up on first view, so
// the very first connection attempt fails a couple seconds in and a retry
// succeeds - see docs/milestone-0-findings.md. Retry a bounded number of
// times before surfacing an error, rather than either hanging forever or
// failing on the first (often transient) hiccup. This budget also covers
// alternating stream extension (see extensionForAttempt) since providers vary
// in whether .ts or .m3u8 is the "right" one.
const MAX_CONNECT_ATTEMPTS = 4;
const RETRY_DELAY_MS = 1500;

// Xtream providers vary in whether .ts or .m3u8 is the "right" extension for
// live streams (see docs/milestone-0-findings.md) - alternate every attempt so
// both get an initial try, then a second chance each, rather than exposing
// this as a manual setting.
const PRIMARY_EXTENSION = '.ts';
const FALLBACK_EXTENSION = '.m3u8';
function extensionForAttempt(attempt: number): string {
  return attempt % 2 === 0 ? PRIMARY_EXTENSION : FALLBACK_EXTENSION;
}
// If mpv never reports 'playback-restart' or a definitive 'end-file' within
// this window, treat the attempt as failed rather than leaving the UI stuck
// on "Connecting..." indefinitely.
const CONNECT_TIMEOUT_MS = 20_000;

let connectTimeoutTimer: ReturnType<typeof setTimeout> | null = null;
// Counts attempts for the channel currently being connected to; reset
// whenever a genuinely new connection is started (a fresh channel pick or a
// manual play() after stop()), not on internal retries.
let connectAttempt = 0;

function stopFallbackPolling() {
  if (fallbackStartTimer) {
    clearTimeout(fallbackStartTimer);
    fallbackStartTimer = null;
  }
  if (fallbackTimer) {
    clearInterval(fallbackTimer);
    fallbackTimer = null;
  }
}

function stopConnectTimeout() {
  if (connectTimeoutTimer) {
    clearTimeout(connectTimeoutTimer);
    connectTimeoutTimer = null;
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

  // Connects to a known URL as-is - no extension probing. Used for
  // reconnecting to a URL that's already known to work (play() after stop(),
  // or recovering from a mid-playback mpv crash), never for a fresh channel
  // selection or a failed-attempt retry (see connectWithProbe for those).
  async function connectToUrl(url: string, streamId: number) {
    stopFallbackPolling();
    stopConnectTimeout();
    stopHeartbeat();
    lastStreamUrl = url;
    set({ status: 'loading', bitrateKbps: null, errorMessage: null, isBuffering: false });
    // Never log `url` itself - it embeds the Xtream username/password
    // (see buildStreamUrl), and this ends up in an exportable log file.
    logInfo(`connecting to channel ${get().currentChannel?.name ?? streamId} (attempt ${connectAttempt + 1}/${MAX_CONNECT_ATTEMPTS})`);
    try {
      await loadUrl(url);
      // Route playback to the chosen output device (mpv keeps this across
      // loads within a session, but reapply each connect so a device picked
      // while stopped, or a fresh mpv process, still honors it). Best-effort:
      // if the saved device is gone, let mpv keep its default rather than fail.
      const audioDevice = useSettingsStore.getState().settings.audioDevice;
      if (audioDevice) {
        await mpvSetProperty('audio-device', audioDevice.name).catch((err) => {
          logWarn(`could not apply audio device ${audioDevice.name}: ${err instanceof Error ? err.message : String(err)}`);
        });
      }
      await mpvSetVolume(get().volume);
      if (get().muted) await mpvSetMute(true);
      // Status flips to 'playing' once mpv reports 'playback-restart' (see
      // initEventListener), which fires only after buffering actually completes.

      connectTimeoutTimer = setTimeout(() => {
        handleFailedAttempt(streamId);
      }, CONNECT_TIMEOUT_MS);

      fallbackStartTimer = setTimeout(() => {
        if (get().bitrateKbps != null || get().currentChannel?.stream_id !== streamId) return;
        fallbackTimer = setInterval(() => {
          if (get().bitrateKbps != null) {
            stopFallbackPolling();
            return;
          }
          getProperty('packet-audio-bitrate');
        }, 2500);
      }, 3000);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      logError(`connect failed for channel ${get().currentChannel?.name ?? streamId}: ${message}`);
      set({ status: 'error', errorMessage: message });
      throw err;
    }
  }

  // Builds the URL for the current connectAttempt (choosing a stream
  // extension via extensionForAttempt) and connects. Used only for a fresh
  // channel selection or a failed-attempt retry - the only paths that should
  // be probing extensions at all.
  async function connectWithProbe(streamId: number) {
    if (!activeCreds) return;
    const url = buildStreamUrl(activeCreds, streamId, extensionForAttempt(connectAttempt));
    await connectToUrl(url, streamId);
  }

  // Called when a connection attempt stalls (CONNECT_TIMEOUT_MS elapses with
  // no 'playback-restart') or mpv reports a definitive failure ('end-file'
  // with reason 'error'). Retries a bounded number of times - see
  // MAX_CONNECT_ATTEMPTS - before giving up and surfacing an error with
  // mpv's own stderr tail attached for diagnosis.
  async function handleFailedAttempt(streamId: number) {
    if (get().currentChannel?.stream_id !== streamId || get().status !== 'loading') return;
    stopConnectTimeout();
    connectAttempt += 1;
    logWarn(`connect attempt ${connectAttempt}/${MAX_CONNECT_ATTEMPTS} failed for channel ${get().currentChannel?.name ?? streamId}`);

    if (connectAttempt < MAX_CONNECT_ATTEMPTS) {
      await new Promise((resolve) => setTimeout(resolve, RETRY_DELAY_MS));
      if (get().currentChannel?.stream_id !== streamId || get().status !== 'loading') return;
      await connectWithProbe(streamId);
      return;
    }

    const tail = await getStderrTail().catch(() => '');
    const suffix = tail ? ` - ${tail}` : '';
    const errorMessage = `Failed to connect after ${MAX_CONNECT_ATTEMPTS} attempts${suffix}`;
    logError(errorMessage);
    set({ status: 'error', errorMessage });
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
      onMpvEvent((event) => {
        // Every mpv IPC message, not just the handful acted on below - the
        // main visibility gap when chasing an intermittent stall that never
        // produces a hard 'error'. Debug level only (enable via Settings >
        // Diagnostics > Verbose logging), so this doesn't spam by default.
        logDebug(`mpv event: ${JSON.stringify(event)}`);

        if (event.event === 'playback-restart') {
          if (get().status === 'loading') {
            const channel = get().currentChannel;
            logInfo(`playback started for channel ${channel?.name ?? channel?.stream_id}`);
            stopConnectTimeout();
            set({ status: 'playing', isBuffering: false });
            setMediaPlayback(true);
            if (channel) startHeartbeat(channel.stream_id);
          }
        } else if (event.event === 'apogee-ipc-closed') {
          // Emitted by the Rust side when mpv's process dies or the IPC
          // socket closes mid-playback (e.g. an mpv crash) - without this,
          // the UI would stay stuck showing 'playing' with dead audio and
          // no error. Reconnect fresh rather than routing through the
          // bounded initial-connect retry counter, since this can happen
          // long after a successful connect.
          const channel = get().currentChannel;
          logError(`mpv connection lost for channel ${channel?.name ?? channel?.stream_id}`);
          stopHeartbeat();
          if ((get().status === 'playing' || get().status === 'loading') && channel && lastStreamUrl) {
            connectAttempt = 0;
            connectToUrl(lastStreamUrl, channel.stream_id);
          }
        } else if (event.event === 'end-file') {
          const channel = get().currentChannel;
          if (event.reason === 'error' && get().status === 'loading' && channel && activeCreds) {
            handleFailedAttempt(channel.stream_id);
          } else if (event.reason && event.reason !== 'eof' && event.reason !== 'stop' && event.reason !== 'quit') {
            logWarn(`unexpected mpv end-file reason "${event.reason}" while status was ${get().status}`);
          }
        } else if (event.event === 'property-change' && event.name === 'audio-bitrate') {
          const bits = typeof event.data === 'number' ? event.data : null;
          if (bits) {
            set({ bitrateKbps: Math.round(bits / 1000) });
            stopFallbackPolling();
          } else {
            set({ bitrateKbps: null });
          }
        } else if (event.request_id === GET_PROPERTY_REQUEST_ID && typeof event.data === 'number') {
          // Fallback reply: packet-audio-bitrate, used when the container never
          // populates audio-bitrate live (see PLAN.md section 5).
          set({ bitrateKbps: Math.round(event.data / 1000) });
          stopFallbackPolling();
        } else if (event.event === 'property-change' && event.name === 'core-idle') {
          // mpv stalled waiting on data mid-playback (e.g. a network hiccup) -
          // distinct from the initial-connect 'loading' status, which already
          // covers first-time buffering via its own spinner/"Connecting…" UI.
          set({ isBuffering: event.data === true });
        }
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
      connectAttempt = 0;
      activeCreds = creds;
      set({ currentChannel: channel });
      await connectWithProbe(channel.stream_id);
    },

    async play() {
      const channel = get().currentChannel;
      if (!channel || !lastStreamUrl) return;
      connectAttempt = 0;
      await connectToUrl(lastStreamUrl, channel.stream_id);
    },

    async stop() {
      logInfo(`stopping channel ${get().currentChannel?.name ?? get().currentChannel?.stream_id}`);
      stopFallbackPolling();
      stopConnectTimeout();
      stopHeartbeat();
      connectAttempt = 0;
      await stopPlayback();
      set({ status: 'stopped', bitrateKbps: null, isBuffering: false });
      await setMediaPlayback(false);
    },

    async setVolume(volume) {
      // Dragging the slider while muted would otherwise look like it's doing
      // nothing (audio stays silent) - unmute so it takes audible effect.
      if (get().muted) {
        set({ muted: false });
        if (get().currentChannel) await mpvSetMute(false);
      }
      set({ volume });
      if (get().currentChannel) {
        await mpvSetVolume(volume);
      }
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
      if (get().currentChannel) {
        await mpvSetMute(next);
      }
    },
  };
});
