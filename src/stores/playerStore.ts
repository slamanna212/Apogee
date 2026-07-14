import { create } from 'zustand';
import type { XtreamChannel } from '../types/xtream';
import type { PlayerState } from '../types/player';
import {
  GET_PROPERTY_REQUEST_ID,
  getProperty,
  getStderrTail,
  loadUrl,
  onMpvEvent,
  stopPlayback,
  setVolume as mpvSetVolume,
} from '../lib/mpvClient';
import { buildStreamUrl, type XtreamCredentials } from '../lib/xtream';
import { onMediaControlEvent, setMediaPlayback } from '../lib/mediaSession';

interface PlayerActions {
  initEventListener: () => void;
  selectChannel: (
    channel: XtreamChannel,
    creds: XtreamCredentials,
    streamExtension: string,
  ) => Promise<void>;
  play: () => Promise<void>;
  stop: () => Promise<void>;
  setVolume: (volume: number) => Promise<void>;
}

type PlayerStore = PlayerState & PlayerActions;

let listening = false;
let fallbackTimer: ReturnType<typeof setInterval> | null = null;
let fallbackStartTimer: ReturnType<typeof setTimeout> | null = null;
// Last connected stream URL, kept around so `play()` can reconnect after a
// `stop()` without needing the channel to be reselected from the list.
let lastStreamUrl: string | null = null;

// Some Xtream providers only spin the upstream channel up on first view, so
// the very first connection attempt fails a couple seconds in and a retry
// succeeds - see docs/milestone-0-findings.md. Retry a bounded number of
// times before surfacing an error, rather than either hanging forever or
// failing on the first (often transient) hiccup.
const MAX_CONNECT_ATTEMPTS = 3;
const RETRY_DELAY_MS = 1500;
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
  async function connect(url: string, streamId: number) {
    stopFallbackPolling();
    stopConnectTimeout();
    lastStreamUrl = url;
    set({ status: 'loading', bitrateKbps: null, errorMessage: null });
    try {
      await loadUrl(url);
      await mpvSetVolume(get().volume);
      // Status flips to 'playing' once mpv reports 'playback-restart' (see
      // initEventListener), which fires only after buffering actually completes.

      connectTimeoutTimer = setTimeout(() => {
        handleFailedAttempt(url, streamId);
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
      set({ status: 'error', errorMessage: message });
      throw err;
    }
  }

  // Called when a connection attempt stalls (CONNECT_TIMEOUT_MS elapses with
  // no 'playback-restart') or mpv reports a definitive failure ('end-file'
  // with reason 'error'). Retries a bounded number of times - see
  // MAX_CONNECT_ATTEMPTS - before giving up and surfacing an error with
  // mpv's own stderr tail attached for diagnosis.
  async function handleFailedAttempt(url: string, streamId: number) {
    if (get().currentChannel?.stream_id !== streamId || get().status !== 'loading') return;
    stopConnectTimeout();
    connectAttempt += 1;

    if (connectAttempt < MAX_CONNECT_ATTEMPTS) {
      await new Promise((resolve) => setTimeout(resolve, RETRY_DELAY_MS));
      if (get().currentChannel?.stream_id !== streamId || get().status !== 'loading') return;
      await connect(url, streamId);
      return;
    }

    const tail = await getStderrTail().catch(() => '');
    const suffix = tail ? ` - ${tail}` : '';
    set({
      status: 'error',
      errorMessage: `Failed to connect after ${MAX_CONNECT_ATTEMPTS} attempts${suffix}`,
    });
  }

  return {
    status: 'idle',
    currentChannel: null,
    volume: 70,
    bitrateKbps: null,
    errorMessage: null,

    initEventListener() {
      if (listening) return;
      listening = true;
      onMpvEvent((event) => {
        if (event.event === 'playback-restart') {
          if (get().status === 'loading') {
            stopConnectTimeout();
            set({ status: 'playing' });
            setMediaPlayback(true);
          }
        } else if (event.event === 'end-file') {
          const channel = get().currentChannel;
          if (event.reason === 'error' && get().status === 'loading' && channel && lastStreamUrl) {
            handleFailedAttempt(lastStreamUrl, channel.stream_id);
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
        }
      });
      onMediaControlEvent((kind) => {
        if (!get().currentChannel) return;
        if (kind === 'play') {
          get().play();
        } else {
          // 'pause' and 'toggle' both mean "stop" - live radio has no pause.
          get().stop();
        }
      });
    },

    async selectChannel(channel, creds, streamExtension) {
      connectAttempt = 0;
      set({ currentChannel: channel });
      const url = buildStreamUrl(creds, channel.stream_id, streamExtension);
      await connect(url, channel.stream_id);
    },

    async play() {
      const channel = get().currentChannel;
      if (!channel || !lastStreamUrl) return;
      connectAttempt = 0;
      await connect(lastStreamUrl, channel.stream_id);
    },

    async stop() {
      stopFallbackPolling();
      stopConnectTimeout();
      connectAttempt = 0;
      await stopPlayback();
      set({ status: 'stopped', bitrateKbps: null });
      await setMediaPlayback(false);
    },

    async setVolume(volume) {
      set({ volume });
      if (get().currentChannel) {
        await mpvSetVolume(volume);
      }
    },
  };
});
