import { DEFAULT_AUDIO_BUFFER, normalizeAudioBuffer, audioBufferError, type AudioBufferSettings } from '../lib/audioBuffer';
import { create } from 'zustand';
import { load, type Store } from '@tauri-apps/plugin-store';
import { getSecret, setSecret, getBuiltinStellarApiKey, SECRET_KEYS } from '../lib/secrets';
import { DEFAULT_EQUALIZER, normalizeEqualizerSettings, type EqualizerSettings } from '../lib/equalizer';
import { migrateDevice, setDevice, setBuffering } from '../lib/playerClient';

export type UpdateChannel = 'stable' | 'beta';
export type StartupPage = 'home' | 'channels' | 'favorites' | 'recent' | 'alerts';
export type HomePageSize = 'small' | 'standard' | 'expanded';

export interface ScrobblingSettings {
  lastfm: {
    enabled: boolean;
  };
}

/** A chosen audio output device. `id` is the CPAL device identifier (the
 *  identity to persist and pass to `player_set_device` - never a display
 *  name, which is not guaranteed unique); `name` is its friendly label.
 *  `null` in settings means "system default". */
export interface AudioDeviceSelection {
  id: string;
  name: string;
}

/** Pre-Symphonia-migration shape of a saved device selection: mpv's
 *  `audio-device-list` name plus a friendly description, with no stable `id`. */
interface LegacyAudioDeviceSelection {
  name: string;
  description: string;
}

function isLegacyAudioDevice(value: unknown): value is LegacyAudioDeviceSelection {
  return (
    typeof value === 'object' &&
    value !== null &&
    'name' in value &&
    'description' in value &&
    !('id' in value)
  );
}

export interface Settings {
  baseUrl: string;
  username: string;
  password: string;
  categoryIds: string[];
  categoryNames: string[];
  volume: number;
  updateChannel: UpdateChannel;
  checkUpdatesOnStartup: boolean;
  startupPage: StartupPage;
  homePageSize: HomePageSize;
  keepMiniWindowOnTop: boolean;
  onboardingComplete: boolean;
  onboardingStep: number;
  verboseLogging: boolean;
  discordRpcEnabled: boolean;
  scrobbling: ScrobblingSettings;
  lastSleepTimerMinutes: number;
  audioDevice: AudioDeviceSelection | null;
  equalizer: EqualizerSettings;
  audioBuffer: AudioBufferSettings;
}

export const DEFAULT_SETTINGS: Settings = {
  baseUrl: '',
  username: '',
  password: '',
  categoryIds: [],
  categoryNames: [],
  volume: 80,
  updateChannel: 'stable',
  checkUpdatesOnStartup: true,
  startupPage: 'home',
  homePageSize: 'standard',
  keepMiniWindowOnTop: true,
  onboardingComplete: false,
  onboardingStep: 0,
  verboseLogging: false,
  discordRpcEnabled: false,
  scrobbling: { lastfm: { enabled: false } },
  lastSleepTimerMinutes: 30,
  audioDevice: null,
  equalizer: DEFAULT_EQUALIZER,
  audioBuffer: DEFAULT_AUDIO_BUFFER,
};

type PersistedSettings = Omit<Settings, 'password'>;

interface SettingsState {
  settings: Settings;
  /** Baked in at build time from the shared StellarTunerLog API key - see secrets.rs. */
  builtinStellarApiKey: string | null;
  loaded: boolean;
  /** Non-null for one load() when a saved MPV device selection couldn't be
   *  carried over to the new device model - see player_migrate_device in
   *  src/lib/playerClient.ts. Settings.tsx surfaces this once, then it should
   *  be cleared via dismissDeviceMigrationNotice(). */
  deviceMigrationNotice: string | null;
  load: () => Promise<void>;
  update: (patch: Partial<Settings>) => Promise<void>;
  dismissDeviceMigrationNotice: () => void;
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
  deviceMigrationNotice: null,
  async load() {
    const store = await getStore();
    const stored = (await store.get<Record<string, unknown>>('settings')) ?? {};

    // Migrate a device selection saved under the old MPV-backed shape
    // ({name, description}, no stable id). Rust matches it against the real CPAL
    // devices and tells us which id it adopted - persist exactly that, or the
    // migration would silently undo itself on the next launch. A null id means it
    // could not be matched and the system default is being used instead, which
    // comes with an explanation to show the user once.
    let deviceMigrationNotice: string | null = null;
    if (isLegacyAudioDevice(stored.audioDevice)) {
      const legacyName = stored.audioDevice.name;
      const migration = await migrateDevice(legacyName).catch(() => null);
      deviceMigrationNotice = migration?.notice ?? null;

      const migrated: AudioDeviceSelection | null = migration?.deviceId
        ? { id: migration.deviceId, name: legacyName }
        : null;

      const { audioDevice: _legacyAudioDevice, ...rest } = stored;
      await store.set('settings', { ...rest, audioDevice: migrated });
      await store.save();
      stored.audioDevice = migrated;
    }

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

    // Migrate the old singular categoryId/categoryName fields (pre-multi-select)
    // into the new array shape so existing users keep seeing their channels.
    const migratedCategoryIds = Array.isArray(stored.categoryIds)
      ? (stored.categoryIds as string[])
      : stored.categoryId
        ? [stored.categoryId as string]
        : DEFAULT_SETTINGS.categoryIds;
    const migratedCategoryNames = Array.isArray(stored.categoryNames)
      ? (stored.categoryNames as string[])
      : stored.categoryName
        ? [stored.categoryName as string]
        : DEFAULT_SETTINGS.categoryNames;

    // Installs from before onboarding existed won't have onboardingComplete in
    // their stored settings - if they already have working Xtream config, treat
    // onboarding as already done rather than replaying it on their next launch.
    const isPreOnboardingInstall = stored.onboardingComplete === undefined && !!stored.baseUrl && !!stored.username && migratedCategoryIds.length > 0;

    // Carry over the old fixed "default volume" setting as the initial
    // remembered volume for installs that predate this being a live value.
    const legacyDefaultVolume = typeof stored.defaultVolume === 'number' ? stored.defaultVolume : undefined;

    set({
      settings: {
        ...DEFAULT_SETTINGS,
        ...(stored as Partial<PersistedSettings>),
        startupPage: ['home', 'channels', 'favorites', 'recent', 'alerts'].includes(stored.startupPage as string)
          ? stored.startupPage as StartupPage : DEFAULT_SETTINGS.startupPage,
        homePageSize: ['small', 'standard', 'expanded'].includes(stored.homePageSize as string)
          ? stored.homePageSize as HomePageSize : DEFAULT_SETTINGS.homePageSize,
        checkUpdatesOnStartup: typeof stored.checkUpdatesOnStartup === 'boolean'
          ? stored.checkUpdatesOnStartup : DEFAULT_SETTINGS.checkUpdatesOnStartup,
        categoryIds: migratedCategoryIds,
        categoryNames: migratedCategoryNames,
        volume: typeof stored.volume === 'number' ? stored.volume : (legacyDefaultVolume ?? DEFAULT_SETTINGS.volume),
        equalizer: normalizeEqualizerSettings(stored.equalizer),
        audioBuffer: normalizeAudioBuffer(stored.audioBuffer),
        password: password ?? '',
        onboardingComplete: isPreOnboardingInstall || Boolean(stored.onboardingComplete),
        audioDevice: (stored.audioDevice as AudioDeviceSelection | null | undefined) ?? null,
      },
      builtinStellarApiKey,
      deviceMigrationNotice,
      loaded: true,
    });
  },
  async update(patch) {
    if (patch.audioBuffer) {
      const error = audioBufferError(patch.audioBuffer);
      if (error) throw new Error(error);
      await setBuffering(patch.audioBuffer);
    }
    const previous = get().settings;
    const next = { ...previous, ...patch };
    set({ settings: next });

    // Apply the output preference immediately, independently of disk persistence.
    // Keeping this here also covers callers other than the Settings picker.
    const deviceUpdate = patch.audioDevice !== undefined
      && (previous.audioDevice?.id ?? null) !== (next.audioDevice?.id ?? null)
      ? setDevice(next.audioDevice?.id ?? null)
      : Promise.resolve();

    await Promise.all([
      deviceUpdate,
      (async () => {
        const { password, ...persisted } = next;
        const store = await getStore();
        await store.set('settings', persisted);
        await store.save();

        if (patch.password !== undefined) {
          await setSecret(SECRET_KEYS.xtreamPassword, password);
        }
      })(),
    ]);
  },
  dismissDeviceMigrationNotice() {
    set({ deviceMigrationNotice: null });
  },
}));
