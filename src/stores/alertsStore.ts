import { create } from 'zustand';
import { load, type Store } from '@tauri-apps/plugin-store';
import type { StellarStation } from '../types/stellarTunerLog';
import type { AlertEntry } from '../types/alerts';
import { matchesEntry, normalizeText, normalizeTitle } from '../lib/songMatcher';
import { sanitizePersistedAlerts, type PersistedAlertsData } from '../lib/alertPersistence';
import { ensureOSPermission, fireAlert } from '../lib/alertNotify';
import { useChannelStore } from './channelStore';

const DEFAULT_ALERTS: PersistedAlertsData = {
  entries: [],
  notifyOS: true,
  notifyInApp: true,
};

interface AlertsState extends PersistedAlertsData {
  loaded: boolean;
  /** streamId -> last-seen normalized song key, for edge detection. Not persisted. */
  lastMatched: Map<number, string>;
  load: () => Promise<void>;
  followTrack: (artist: string, title: string) => Promise<void>;
  followArtist: (artist: string) => Promise<void>;
  unfollow: (id: string) => Promise<void>;
  setNotifyOS: (value: boolean) => Promise<void>;
  setNotifyInApp: (value: boolean) => Promise<void>;
  scan: (
    nowPlaying: Map<number, StellarStation>,
    allowInAppNotifications: boolean,
    onGoToChannel?: (streamId: number) => void,
  ) => void;
}

let storePromise: Promise<Store> | null = null;
function getStore() {
  if (!storePromise) {
    storePromise = load('alerts.json', { autoSave: false, defaults: {} });
  }
  return storePromise;
}

async function persist(next: PersistedAlertsData) {
  const store = await getStore();
  await store.set('alerts', next);
  await store.save();
}

function songKey(station: StellarStation): string {
  return `${normalizeText(station.artist)}|${normalizeTitle(station.title)}`;
}

export const useAlertsStore = create<AlertsState>((set, get) => ({
  ...DEFAULT_ALERTS,
  loaded: false,
  lastMatched: new Map(),
  async load() {
    const store = await getStore();
    const stored = await store.get<unknown>('alerts');
    set({ ...sanitizePersistedAlerts(stored), loaded: true });
  },
  async followTrack(artist, title) {
    if (typeof artist !== 'string' || !artist.trim() || typeof title !== 'string' || !title.trim()) return;
    const { entries, notifyOS, notifyInApp } = get();
    const entry: AlertEntry = { id: crypto.randomUUID(), type: 'track', artist, title, createdAt: Date.now() };
    const next = [...entries, entry];
    set({ entries: next });
    await persist({ entries: next, notifyOS, notifyInApp });
  },
  async followArtist(artist) {
    if (typeof artist !== 'string' || !artist.trim()) return;
    const { entries, notifyOS, notifyInApp } = get();
    const entry: AlertEntry = { id: crypto.randomUUID(), type: 'artist', artist, createdAt: Date.now() };
    const next = [...entries, entry];
    set({ entries: next });
    await persist({ entries: next, notifyOS, notifyInApp });
  },
  async unfollow(id) {
    const { entries, notifyOS, notifyInApp } = get();
    const next = entries.filter((e) => e.id !== id);
    set({ entries: next });
    await persist({ entries: next, notifyOS, notifyInApp });
  },
  async setNotifyOS(value) {
    if (value && !(await ensureOSPermission())) return;
    const { entries, notifyInApp } = get();
    set({ notifyOS: value });
    await persist({ entries, notifyOS: value, notifyInApp });
  },
  async setNotifyInApp(value) {
    const { entries, notifyOS } = get();
    set({ notifyInApp: value });
    await persist({ entries, notifyOS, notifyInApp: value });
  },
  scan(nowPlaying, allowInAppNotifications, onGoToChannel) {
    const { entries, notifyOS, notifyInApp, lastMatched } = get();
    if (entries.length === 0) return;
    const channels = useChannelStore.getState().channels;

    for (const [streamId, station] of nowPlaying) {
      const key = songKey(station);
      if (lastMatched.get(streamId) === key) continue;
      lastMatched.set(streamId, key);

      const match = entries.find((entry) => matchesEntry(station, entry));
      if (match) {
        const channel = channels.find((c) => c.stream_id === streamId);
        const channelName = channel?.name ?? station.name;
        const artworkUrl = station.artwork_url || channel?.stream_icon;
        void fireAlert(
          match,
          station,
          streamId,
          channelName,
          artworkUrl,
          notifyOS,
          notifyInApp && allowInAppNotifications,
          onGoToChannel,
        );
      }
    }
  },
}));
