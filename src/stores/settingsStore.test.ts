import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockStore = vi.hoisted(() => ({
  data: {} as Record<string, unknown>,
  get: vi.fn(async (key: string) => mockStore.data[key]),
  set: vi.fn(async (key: string, value: unknown) => {
    mockStore.data[key] = value;
  }),
  save: vi.fn(async () => {}),
}));

vi.mock('@tauri-apps/plugin-store', () => ({
  load: vi.fn(async () => mockStore),
}));

vi.mock('../lib/secrets', () => ({
  getSecret: vi.fn(async () => null),
  setSecret: vi.fn(async () => {}),
  getBuiltinStellarApiKey: vi.fn(async () => null),
  SECRET_KEYS: { xtreamPassword: 'xtream_password' },
}));

vi.mock('../lib/playerClient', () => ({
  migrateDevice: vi.fn(async () => ({ deviceId: null, notice: null })),
}));

import { migrateDevice } from '../lib/playerClient';
import { useSettingsStore } from './settingsStore';

beforeEach(() => {
  mockStore.data = {};
  vi.clearAllMocks();
  useSettingsStore.setState({ loaded: false });
});

describe('settingsStore migration of categoryId/categoryName', () => {
  it('defaults to empty arrays on a fresh install', async () => {
    mockStore.data.settings = {};
    await useSettingsStore.getState().load();
    expect(useSettingsStore.getState().settings.categoryIds).toEqual([]);
    expect(useSettingsStore.getState().settings.categoryNames).toEqual([]);
  });

  it('wraps a legacy singular categoryId/categoryName into a one-element array', async () => {
    mockStore.data.settings = {
      baseUrl: 'http://example.com',
      username: 'alice',
      categoryId: '5',
      categoryName: 'SiriusXM',
    };
    await useSettingsStore.getState().load();
    expect(useSettingsStore.getState().settings.categoryIds).toEqual(['5']);
    expect(useSettingsStore.getState().settings.categoryNames).toEqual(['SiriusXM']);
  });

  it('passes through an existing categoryIds array unchanged and ignores any legacy categoryId', async () => {
    mockStore.data.settings = {
      categoryId: '5',
      categoryName: 'SiriusXM',
      categoryIds: ['1', '2'],
      categoryNames: ['Group 1', 'Group 2'],
    };
    await useSettingsStore.getState().load();
    expect(useSettingsStore.getState().settings.categoryIds).toEqual(['1', '2']);
    expect(useSettingsStore.getState().settings.categoryNames).toEqual(['Group 1', 'Group 2']);
  });
});

describe('settingsStore migration of a legacy MPV audio device selection', () => {
  it('passes an already-migrated {id, name} selection through unchanged and never calls player_migrate_device', async () => {
    mockStore.data.settings = { audioDevice: { id: 'cpal-device-1', name: 'Speakers' } };
    await useSettingsStore.getState().load();
    expect(useSettingsStore.getState().settings.audioDevice).toEqual({ id: 'cpal-device-1', name: 'Speakers' });
    expect(migrateDevice).not.toHaveBeenCalled();
    expect(useSettingsStore.getState().deviceMigrationNotice).toBeNull();
  });

  it('resets a legacy {name, description} selection to system default and calls player_migrate_device with its name', async () => {
    mockStore.data.settings = { audioDevice: { name: 'alsa/hw:0,0', description: 'Built-in Audio' } };
    await useSettingsStore.getState().load();
    expect(migrateDevice).toHaveBeenCalledExactlyOnceWith('alsa/hw:0,0');
    expect(useSettingsStore.getState().settings.audioDevice).toBeNull();
    // The legacy value is also scrubbed from disk, not just in-memory.
    expect(mockStore.data.settings).toMatchObject({ audioDevice: null });
  });

  it('persists the device id Rust matched, so a successful migration survives a restart', async () => {
    // Rust adopts the matched device in memory. If we persisted null here instead, the
    // engine and the saved settings would disagree and the migration would silently
    // undo itself on the next launch.
    vi.mocked(migrateDevice).mockResolvedValueOnce({ deviceId: 'alsa:pipewire', notice: null });
    mockStore.data.settings = { audioDevice: { name: 'pulse/pipewire', description: 'PipeWire' } };
    await useSettingsStore.getState().load();

    expect(useSettingsStore.getState().settings.audioDevice).toEqual({
      id: 'alsa:pipewire',
      name: 'pulse/pipewire',
    });
    expect(mockStore.data.settings).toMatchObject({
      audioDevice: { id: 'alsa:pipewire' },
    });
    expect(useSettingsStore.getState().deviceMigrationNotice).toBeNull();
  });

  it('surfaces the explanation returned by player_migrate_device as deviceMigrationNotice', async () => {
    vi.mocked(migrateDevice).mockResolvedValueOnce({
      deviceId: null,
      notice: 'Your saved USB headset was not found; using system default.',
    });
    mockStore.data.settings = { audioDevice: { name: 'usb-headset', description: 'USB Headset' } };
    await useSettingsStore.getState().load();
    expect(useSettingsStore.getState().deviceMigrationNotice).toBe('Your saved USB headset was not found; using system default.');
  });

  it('leaves deviceMigrationNotice null when there was nothing to migrate', async () => {
    mockStore.data.settings = {};
    await useSettingsStore.getState().load();
    expect(useSettingsStore.getState().settings.audioDevice).toBeNull();
    expect(migrateDevice).not.toHaveBeenCalled();
    expect(useSettingsStore.getState().deviceMigrationNotice).toBeNull();
  });

  it('dismissDeviceMigrationNotice clears the notice without touching other settings', async () => {
    vi.mocked(migrateDevice).mockResolvedValueOnce({
      deviceId: null,
      notice: 'Your saved device was not found; using system default.',
    });
    mockStore.data.settings = { audioDevice: { name: 'alsa/hw:0,0', description: 'Built-in Audio' } };
    await useSettingsStore.getState().load();
    expect(useSettingsStore.getState().deviceMigrationNotice).not.toBeNull();

    useSettingsStore.getState().dismissDeviceMigrationNotice();
    expect(useSettingsStore.getState().deviceMigrationNotice).toBeNull();
    expect(useSettingsStore.getState().settings.audioDevice).toBeNull();
  });
});
