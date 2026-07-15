import { create } from 'zustand';
import { load, type Store } from '@tauri-apps/plugin-store';
import { getSecret, setSecret, getBuiltinStellarApiKey, SECRET_KEYS } from '../lib/secrets';

export type UpdateChannel = 'stable' | 'beta';

export interface Settings {
  baseUrl: string;
  username: string;
  password: string;
  streamExtension: string;
  categoryId: string | null;
  categoryName: string | null;
  pollIntervalSec: number;
  defaultVolume: number;
  updateChannel: UpdateChannel;
  keepMiniWindowOnTop: boolean;
  onboardingComplete: boolean;
  onboardingStep: number;
  verboseLogging: boolean;
  discordRpcEnabled: boolean;
}

export const DEFAULT_SETTINGS: Settings = {
  baseUrl: '',
  username: '',
  password: '',
  streamExtension: '.ts',
  categoryId: null,
  categoryName: null,
  pollIntervalSec: 10,
  defaultVolume: 70,
  updateChannel: 'stable',
  keepMiniWindowOnTop: true,
  onboardingComplete: false,
  onboardingStep: 0,
  verboseLogging: false,
  discordRpcEnabled: false,
};

type PersistedSettings = Omit<Settings, 'password'>;

interface SettingsState {
  settings: Settings;
  /** Baked in at build time from the shared StellarTunerLog API key - see secrets.rs. */
  builtinStellarApiKey: string | null;
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
  builtinStellarApiKey: null,
  loaded: false,
  async load() {
    const store = await getStore();
    const stored = (await store.get<Record<string, unknown>>('settings')) ?? {};

    // Migrate any plaintext password left over from before keyring storage was added.
    const legacyPassword = typeof stored.password === 'string' ? stored.password : undefined;

    let [password, builtinStellarApiKey] = await Promise.all([
      getSecret(SECRET_KEYS.xtreamPassword),
      getBuiltinStellarApiKey(),
    ]);

    if (!password && legacyPassword) {
      await setSecret(SECRET_KEYS.xtreamPassword, legacyPassword);
      password = legacyPassword;
    }

    if (legacyPassword !== undefined) {
      const { password: _password, ...rest } = stored;
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
        onboardingComplete: isPreOnboardingInstall || Boolean(stored.onboardingComplete),
      },
      builtinStellarApiKey,
      loaded: true,
    });
  },
  async update(patch) {
    const next = { ...get().settings, ...patch };
    set({ settings: next });

    const { password, ...persisted } = next;
    const store = await getStore();
    await store.set('settings', persisted);
    await store.save();

    if (patch.password !== undefined) {
      await setSecret(SECRET_KEYS.xtreamPassword, password);
    }
  },
}));
