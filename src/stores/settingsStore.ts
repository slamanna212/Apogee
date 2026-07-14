import { create } from 'zustand';
import { load, type Store } from '@tauri-apps/plugin-store';
import { getSecret, setSecret, SECRET_KEYS } from '../lib/secrets';

export type UpdateChannel = 'stable' | 'beta';

export interface Settings {
  baseUrl: string;
  username: string;
  password: string;
  streamExtension: string;
  categoryId: string | null;
  categoryName: string | null;
  stellarApiKey: string;
  pollIntervalSec: number;
  defaultVolume: number;
  updateChannel: UpdateChannel;
  keepMiniWindowOnTop: boolean;
  onboardingComplete: boolean;
  onboardingStep: number;
  verboseLogging: boolean;
}

export const DEFAULT_SETTINGS: Settings = {
  baseUrl: '',
  username: '',
  password: '',
  streamExtension: '.ts',
  categoryId: null,
  categoryName: null,
  stellarApiKey: '',
  pollIntervalSec: 10,
  defaultVolume: 70,
  updateChannel: 'stable',
  keepMiniWindowOnTop: true,
  onboardingComplete: false,
  onboardingStep: 0,
  verboseLogging: false,
};

type PersistedSettings = Omit<Settings, 'password' | 'stellarApiKey'>;

interface SettingsState {
  settings: Settings;
  loaded: boolean;
  load: () => Promise<void>;
  update: (patch: Partial<Settings>) => Promise<void>;
}

let storePromise: Promise<Store> | null = null;
function getStore() {
  if (!storePromise) {
    storePromise = load('settings.json', { autoSave: false, defaults: {} });
  }
  return storePromise;
}

export const useSettingsStore = create<SettingsState>((set, get) => ({
  settings: DEFAULT_SETTINGS,
  loaded: false,
  async load() {
    const store = await getStore();
    const stored = (await store.get<Record<string, unknown>>('settings')) ?? {};

    // Migrate any plaintext secrets left over from before keyring storage was added.
    const legacyPassword = typeof stored.password === 'string' ? stored.password : undefined;
    const legacyStellarApiKey =
      typeof stored.stellarApiKey === 'string' ? stored.stellarApiKey : undefined;

    let [password, stellarApiKey] = await Promise.all([
      getSecret(SECRET_KEYS.xtreamPassword),
      getSecret(SECRET_KEYS.stellarApiKey),
    ]);

    if (!password && legacyPassword) {
      await setSecret(SECRET_KEYS.xtreamPassword, legacyPassword);
      password = legacyPassword;
    }
    if (!stellarApiKey && legacyStellarApiKey) {
      await setSecret(SECRET_KEYS.stellarApiKey, legacyStellarApiKey);
      stellarApiKey = legacyStellarApiKey;
    }

    if (legacyPassword !== undefined || legacyStellarApiKey !== undefined) {
      const { password: _password, stellarApiKey: _stellarApiKey, ...rest } = stored;
      await store.set('settings', rest);
      await store.save();
    }

    // Installs from before onboarding existed won't have onboardingComplete in
    // their stored settings - if they already have working Xtream config, treat
    // onboarding as already done rather than replaying it on their next launch.
    const isPreOnboardingInstall = stored.onboardingComplete === undefined && !!stored.baseUrl && !!stored.username && !!stored.categoryId;

    set({
      settings: {
        ...DEFAULT_SETTINGS,
        ...(stored as Partial<PersistedSettings>),
        password: password ?? '',
        stellarApiKey: stellarApiKey ?? '',
        onboardingComplete: isPreOnboardingInstall || Boolean(stored.onboardingComplete),
      },
      loaded: true,
    });
  },
  async update(patch) {
    const next = { ...get().settings, ...patch };
    set({ settings: next });

    const { password, stellarApiKey, ...persisted } = next;
    const store = await getStore();
    await store.set('settings', persisted);
    await store.save();

    if (patch.password !== undefined) {
      await setSecret(SECRET_KEYS.xtreamPassword, password);
    }
    if (patch.stellarApiKey !== undefined) {
      await setSecret(SECRET_KEYS.stellarApiKey, stellarApiKey);
    }
  },
}));
