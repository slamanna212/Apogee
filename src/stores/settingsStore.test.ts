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
