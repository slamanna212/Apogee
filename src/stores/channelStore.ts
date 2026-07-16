import { create } from 'zustand';
import { load, type Store } from '@tauri-apps/plugin-store';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarStation } from '../types/stellarTunerLog';
import { getLiveStreams, type XtreamCredentials } from '../lib/xtream';
import { getChannels, getNowPlaying } from '../lib/stellarTunerLog';
import { buildChannelMetadataMap, buildNowPlayingMap, nowPlayingMapsEqual } from '../lib/channelMatcher';

const POLL_INTERVAL_MS = 10_000;
const MAX_POLL_INTERVAL_MS = 2 * 60_000;

/** Exponential backoff (capped) on consecutive StellarTunerLog poll failures, reset to the base interval on the next success. */
export function nextPollDelayMs(failureCount: number): number {
  if (failureCount <= 0) return POLL_INTERVAL_MS;
  return Math.min(POLL_INTERVAL_MS * 2 ** failureCount, MAX_POLL_INTERVAL_MS);
}

// Persists across channel-list/category changes for the lifetime of the app -
// stale entries for stream_ids no longer in `channels` are simply never read.
const stationIdCache = new Map<number, string>();

// The StellarTunerLog channel catalog rarely changes - avoid refetching it on
// every launch by caching it to disk for a few hours.
const CHANNEL_CACHE_TTL_MS = 6 * 60 * 60 * 1000;

interface StellarChannelsCache {
  channels: StellarChannel[];
  fetchedAt: number;
}

let cacheStorePromise: Promise<Store> | null = null;
function getCacheStore() {
  if (!cacheStorePromise) {
    cacheStorePromise = load('stellar-channels-cache.json', { autoSave: false, defaults: {} });
  }
  return cacheStorePromise;
}

async function getCachedStellarChannels(): Promise<StellarChannel[]> {
  const store = await getCacheStore();
  const cached = await store.get<StellarChannelsCache>('stellarChannels');
  if (cached && Date.now() - cached.fetchedAt < CHANNEL_CACHE_TTL_MS) {
    return cached.channels;
  }
  const channels = await getChannels();
  await store.set('stellarChannels', { channels, fetchedAt: Date.now() } satisfies StellarChannelsCache);
  await store.save();
  return channels;
}

interface ChannelState {
  channels: XtreamChannel[];
  status: 'idle' | 'loading' | 'loaded' | 'error';
  error: string | null;
  nowPlaying: Map<number, StellarStation>;
  channelMetadata: Map<number, StellarChannel>;
  metadataStatus: 'idle' | 'loading' | 'loaded' | 'error';
  pollFailureCount: number;
  fetchChannels: (creds: XtreamCredentials, categoryId: string) => Promise<void>;
  pollNowPlaying: (apiKey?: string) => Promise<void>;
  fetchChannelMetadata: () => Promise<void>;
}

export const useChannelStore = create<ChannelState>((set, get) => ({
  channels: [],
  status: 'idle',
  error: null,
  nowPlaying: new Map(),
  channelMetadata: new Map(),
  metadataStatus: 'idle',
  pollFailureCount: 0,
  async fetchChannels(creds, categoryId) {
    set({ status: 'loading', error: null });
    try {
      const channels = await getLiveStreams(creds, categoryId);
      channels.sort((a, b) => a.num - b.num);
      set({ channels, status: 'loaded' });
    } catch (err) {
      set({ status: 'error', error: err instanceof Error ? err.message : String(err) });
    }
  },
  async pollNowPlaying(apiKey) {
    const { channels, nowPlaying } = get();
    if (channels.length === 0) return;
    try {
      const response = await getNowPlaying(apiKey);
      const stations = Object.values(response.stations);
      const next = buildNowPlayingMap(channels, stations, stationIdCache);
      set({
        nowPlaying: nowPlayingMapsEqual(nowPlaying, next) ? nowPlaying : next,
        pollFailureCount: 0,
      });
    } catch {
      // transient poll failure - keep showing the last known now-playing data,
      // but track it so the caller can back off instead of polling at a fixed
      // rate through an outage
      set((s) => ({ pollFailureCount: s.pollFailureCount + 1 }));
    }
  },
  async fetchChannelMetadata() {
    const { channels, metadataStatus } = get();
    if (channels.length === 0 || metadataStatus === 'loading') return;
    set({ metadataStatus: 'loading' });
    try {
      const stellarChannels = await getCachedStellarChannels();
      set({
        channelMetadata: buildChannelMetadataMap(channels, stellarChannels),
        metadataStatus: 'loaded',
      });
    } catch {
      // channel metadata is an enhancement (categories/logos/description/socials) -
      // the app remains usable from Xtream data alone if this fails
      set({ metadataStatus: 'error' });
    }
  },
}));
