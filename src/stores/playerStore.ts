import { create } from 'zustand';
import type { XtreamChannel } from '../types/xtream';
import type { PlayerState } from '../types/player';
import { loadUrl, onMpvEvent, setPause, setVolume as mpvSetVolume } from '../lib/mpvClient';
import { buildStreamUrl, type XtreamCredentials } from '../lib/xtream';

interface PlayerActions {
  initEventListener: () => void;
  selectChannel: (
    channel: XtreamChannel,
    creds: XtreamCredentials,
    streamExtension: string,
  ) => Promise<void>;
  togglePause: () => Promise<void>;
  setVolume: (volume: number) => Promise<void>;
}

type PlayerStore = PlayerState & PlayerActions;

let listening = false;

export const usePlayerStore = create<PlayerStore>((set, get) => ({
  status: 'idle',
  currentChannel: null,
  volume: 70,
  bitrateKbps: null,

  initEventListener() {
    if (listening) return;
    listening = true;
    onMpvEvent((event) => {
      if (event.event === 'property-change' && event.name === 'audio-bitrate') {
        const bits = typeof event.data === 'number' ? event.data : null;
        set({ bitrateKbps: bits ? Math.round(bits / 1000) : null });
      }
    });
  },

  async selectChannel(channel, creds, streamExtension) {
    set({ status: 'loading', currentChannel: channel, bitrateKbps: null });
    try {
      const url = buildStreamUrl(creds, channel.stream_id, streamExtension);
      await loadUrl(url);
      await mpvSetVolume(get().volume);
      set({ status: 'playing' });
    } catch (err) {
      set({ status: 'error' });
      throw err;
    }
  },

  async togglePause() {
    const isPlaying = get().status === 'playing';
    await setPause(isPlaying);
    set({ status: isPlaying ? 'paused' : 'playing' });
  },

  async setVolume(volume) {
    set({ volume });
    if (get().currentChannel) {
      await mpvSetVolume(volume);
    }
  },
}));
