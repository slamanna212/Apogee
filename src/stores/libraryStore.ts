import { create } from 'zustand';
import { load, type Store } from '@tauri-apps/plugin-store';

export type SortMode = 'az' | 'channel_number';
export type ThemeMode = 'system' | 'dark' | 'light';
export type ViewMode = 'grid' | 'list';

interface PersistedLibrary {
  favorites: number[];
  recentlyPlayed: number[];
  sortMode: SortMode;
  themeMode: ThemeMode;
  viewMode: ViewMode;
}

const DEFAULT_LIBRARY: PersistedLibrary = {
  favorites: [],
  recentlyPlayed: [],
  sortMode: 'channel_number',
  themeMode: 'system',
  viewMode: 'list',
};

interface LibraryState extends PersistedLibrary {
  loaded: boolean;
  load: () => Promise<void>;
  toggleFavorite: (streamId: number) => Promise<void>;
  recordPlay: (streamId: number) => Promise<void>;
  setSortMode: (mode: SortMode) => Promise<void>;
  setThemeMode: (mode: ThemeMode) => Promise<void>;
  setViewMode: (mode: ViewMode) => Promise<void>;
}

let storePromise: Promise<Store> | null = null;
function getStore() {
  if (!storePromise) {
    storePromise = load('library.json', { autoSave: false, defaults: {} });
  }
  return storePromise;
}

async function persist(next: PersistedLibrary) {
  const store = await getStore();
  await store.set('library', next);
  await store.save();
}

export const useLibraryStore = create<LibraryState>((set, get) => ({
  ...DEFAULT_LIBRARY,
  loaded: false,
  async load() {
    const store = await getStore();
    const stored = (await store.get<Partial<PersistedLibrary>>('library')) ?? {};
    set({ ...DEFAULT_LIBRARY, ...stored, loaded: true });
  },
  async toggleFavorite(streamId) {
    const { favorites, recentlyPlayed, sortMode, themeMode, viewMode } = get();
    const next = favorites.includes(streamId)
      ? favorites.filter((id) => id !== streamId)
      : [...favorites, streamId];
    set({ favorites: next });
    await persist({ favorites: next, recentlyPlayed, sortMode, themeMode, viewMode });
  },
  async recordPlay(streamId) {
    const { favorites, recentlyPlayed, sortMode, themeMode, viewMode } = get();
    const next = [streamId, ...recentlyPlayed.filter((id) => id !== streamId)];
    set({ recentlyPlayed: next });
    await persist({ favorites, recentlyPlayed: next, sortMode, themeMode, viewMode });
  },
  async setSortMode(mode) {
    const { favorites, recentlyPlayed, themeMode, viewMode } = get();
    set({ sortMode: mode });
    await persist({ favorites, recentlyPlayed, sortMode: mode, themeMode, viewMode });
  },
  async setThemeMode(mode) {
    const { favorites, recentlyPlayed, sortMode, viewMode } = get();
    set({ themeMode: mode });
    await persist({ favorites, recentlyPlayed, sortMode, themeMode: mode, viewMode });
  },
  async setViewMode(mode) {
    const { favorites, recentlyPlayed, sortMode, themeMode } = get();
    set({ viewMode: mode });
    await persist({ favorites, recentlyPlayed, sortMode, themeMode, viewMode: mode });
  },
}));
