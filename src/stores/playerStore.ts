import { create } from 'zustand';
import type { XtreamChannel } from '../types/xtream';
import type { PlayerState } from '../types/player';
import {
  GET_PROPERTY_REQUEST_ID,
  getProperty,
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

export const usePlayerStore = create<PlayerStore>((set, get) => {
  async function connect(url: string, streamId: number) {
    stopFallbackPolling();
    lastStreamUrl = url;
    set({ status: 'loading', bitrateKbps: null });
    try {
      await loadUrl(url);
      await mpvSetVolume(get().volume);
      // Status flips to 'playing' once mpv reports 'playback-restart' (see
      // initEventListener), which fires only after buffering actually completes.

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
      set({ status: 'error' });
      throw err;
    }
  }

  return {
    status: 'idle',
    currentChannel: null,
    volume: 70,
    bitrateKbps: null,

    initEventListener() {
      if (listening) return;
      listening = true;
      onMpvEvent((event) => {
        if (event.event === 'playback-restart') {
          if (get().status === 'loading') {
            set({ status: 'playing' });
            setMediaPlayback(true);
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
      set({ currentChannel: channel });
      const url = buildStreamUrl(creds, channel.stream_id, streamExtension);
      await connect(url, channel.stream_id);
    },

    async play() {
      const channel = get().currentChannel;
      if (!channel || !lastStreamUrl) return;
      await connect(lastStreamUrl, channel.stream_id);
    },

    async stop() {
      stopFallbackPolling();
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
