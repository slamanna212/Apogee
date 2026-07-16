import { create } from 'zustand';
import { openUrl } from '@tauri-apps/plugin-opener';
import {
  asLastFmError,
  beginLastFmAuth,
  completeLastFmAuth,
  disconnectLastFm as disconnectLastFmApi,
  getLastFmConnectionStatus,
} from '../lib/lastfm';
import type { LastFmConnectionStatus } from '../types/lastfm';

interface ProviderConnectionState extends LastFmConnectionStatus {
  status: 'idle' | 'loading' | 'authorizing' | 'connected' | 'error';
  error: string | null;
}

interface ScrobblingState {
  providers: {
    lastfm: ProviderConnectionState;
  };
  pendingLastFmToken: string | null;
  load: () => Promise<void>;
  beginLastFmConnection: () => Promise<void>;
  finishLastFmConnection: () => Promise<void>;
  disconnectLastFm: () => Promise<void>;
  markLastFmDisconnected: (message: string) => void;
}

const EMPTY_LASTFM: ProviderConnectionState = {
  available: false,
  connected: false,
  username: null,
  status: 'idle',
  error: null,
};

function withStatus(connection: LastFmConnectionStatus): ProviderConnectionState {
  return {
    ...connection,
    status: connection.connected ? 'connected' : 'idle',
    error: null,
  };
}

export const useScrobblingStore = create<ScrobblingState>((set, get) => ({
  providers: { lastfm: EMPTY_LASTFM },
  pendingLastFmToken: null,
  async load() {
    set((state) => ({ providers: { ...state.providers, lastfm: { ...state.providers.lastfm, status: 'loading', error: null } } }));
    try {
      const connection = await getLastFmConnectionStatus();
      set({ providers: { lastfm: withStatus(connection) } });
    } catch (error) {
      const parsed = asLastFmError(error);
      set({ providers: { lastfm: { ...EMPTY_LASTFM, status: 'error', error: parsed.message } } });
    }
  },
  async beginLastFmConnection() {
    set((state) => ({ providers: { ...state.providers, lastfm: { ...state.providers.lastfm, status: 'loading', error: null } } }));
    try {
      const auth = await beginLastFmAuth();
      await openUrl(auth.authorizationUrl);
      set((state) => ({
        pendingLastFmToken: auth.token,
        providers: { ...state.providers, lastfm: { ...state.providers.lastfm, status: 'authorizing', error: null } },
      }));
    } catch (error) {
      const parsed = asLastFmError(error);
      set((state) => ({ providers: { ...state.providers, lastfm: { ...state.providers.lastfm, status: 'error', error: parsed.message } } }));
    }
  },
  async finishLastFmConnection() {
    const token = get().pendingLastFmToken;
    if (!token) return;
    set((state) => ({ providers: { ...state.providers, lastfm: { ...state.providers.lastfm, status: 'loading', error: null } } }));
    try {
      const connection = await completeLastFmAuth(token);
      set({ pendingLastFmToken: null, providers: { lastfm: withStatus(connection) } });
    } catch (error) {
      const parsed = asLastFmError(error);
      set((state) => ({
        providers: {
          ...state.providers,
          lastfm: { ...state.providers.lastfm, status: 'authorizing', error: parsed.message },
        },
      }));
    }
  },
  async disconnectLastFm() {
    try {
      await disconnectLastFmApi();
      set((state) => ({
        pendingLastFmToken: null,
        providers: {
          ...state.providers,
          lastfm: { ...state.providers.lastfm, connected: false, username: null, status: 'idle', error: null },
        },
      }));
    } catch (error) {
      const parsed = asLastFmError(error);
      set((state) => ({ providers: { ...state.providers, lastfm: { ...state.providers.lastfm, status: 'error', error: parsed.message } } }));
    }
  },
  markLastFmDisconnected(message) {
    set((state) => ({
      providers: {
        ...state.providers,
        lastfm: { ...state.providers.lastfm, connected: false, username: null, status: 'error', error: message },
      },
    }));
  },
}));
