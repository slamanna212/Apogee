import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { XtreamChannel } from '../types/xtream';
import type { XtreamCredentials } from '../lib/xtream';
import type { Snapshot } from '../lib/playerClient';
import {
  onSnapshot,
  play as playerPlay,
  setMuted as playerSetMuted,
  setVolume as playerSetVolume,
  stop as playerStop,
} from '../lib/playerClient';
import { onMediaControlEvent, setMediaPlayback, setMediaVolume } from '../lib/mediaSession';
import { usePlayerStore } from './playerStore';

const settingsUpdate = vi.hoisted(() => vi.fn());
const mockSettings = vi.hoisted(() => ({
  verboseLogging: false,
}));

vi.mock('@tauri-apps/plugin-log', () => ({
  debug: vi.fn(),
  info: vi.fn(),
  warn: vi.fn(),
  error: vi.fn(),
}));
vi.mock('../lib/playerClient', () => ({
  onSnapshot: vi.fn().mockResolvedValue(() => {}),
  play: vi.fn(),
  stop: vi.fn(),
  setVolume: vi.fn().mockResolvedValue(undefined),
  setMuted: vi.fn().mockResolvedValue(undefined),
  setEqualizer: vi.fn().mockResolvedValue(undefined),
  listDevices: vi.fn().mockResolvedValue([]),
  setDevice: vi.fn().mockResolvedValue(undefined),
  migrateDevice: vi.fn().mockResolvedValue(null),
  getSnapshot: vi.fn(),
}));
vi.mock('../lib/mediaSession', () => ({
  onMediaControlEvent: vi.fn(),
  setMediaPlayback: vi.fn().mockResolvedValue(undefined),
  setMediaVolume: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('../lib/waveform', () => ({
  setWaveformActive: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('./settingsStore', () => ({
  useSettingsStore: { getState: () => ({
    update: settingsUpdate,
    settings: mockSettings,
  }) },
}));

const creds: XtreamCredentials = {
  baseUrl: 'http://example.com:8080',
  username: 'user',
  password: 'pass',
};

const channel: XtreamChannel = {
  stream_id: 42,
  name: 'Octane',
  stream_icon: '',
  num: 1,
  category_id: '1',
};

// Builds a snapshot with sane defaults; each test overrides only what it cares
// about. `revision` must be supplied explicitly by call sites that care about
// ordering - tests that don't care get an auto-incrementing one.
let nextRevision = 1;
function snapshot(overrides: Partial<Snapshot> = {}): Snapshot {
  const revision = overrides.revision ?? nextRevision;
  // Keep the counter strictly ahead of any explicitly-assigned revision too
  // (not just auto-assigned ones), so a later test's auto-assigned revisions
  // can never be lower than an earlier test's explicit ones - the store's
  // own last-applied-revision tracker is a module-level singleton that is
  // never reset between tests (see the comment in beforeEach below).
  if (revision >= nextRevision) nextRevision = revision + 1;
  return {
    generation: 1,
    stationId: String(channel.stream_id),
    state: 'connecting',
    bufferingReason: null,
    attempt: 0,
    bitrateKbps: null,
    sampleRate: null,
    device: null,
    error: null,
    ...overrides,
    revision,
  };
}

// Registered once - the store guards against double registration, so capture
// the callback a single time and reuse it across tests. Captured before any
// beforeEach can clearAllMocks() the call history, so this also proves the
// listener is registered before any command could ever be issued.
usePlayerStore.getState().initEventListener();
const listenerRegisteredBeforeAnyCommand = vi.mocked(onSnapshot).mock.calls.length > 0;
const emitSnapshot = vi.mocked(onSnapshot).mock.calls[0][0];
const emitMediaControl = vi.mocked(onMediaControlEvent).mock.calls[0][0];

async function flushAsync() {
  await vi.advanceTimersByTimeAsync(0);
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllTimers();
  vi.clearAllMocks();
  // Deliberately NOT reset per test: the store's own last-applied-revision
  // tracker (see playerStore.ts) is a module-level singleton that persists
  // across tests too, so revisions handed out here must keep climbing or a
  // later test's snapshots would be rejected as stale against an earlier one.
  mockSettings.verboseLogging = false;
  vi.mocked(playerPlay).mockImplementation(async () => snapshot({ state: 'connecting' }));
  vi.mocked(playerStop).mockImplementation(async () => snapshot({ state: 'stopped', stationId: null }));
  usePlayerStore.setState({
    status: 'idle',
    currentChannel: null,
    volume: 80,
    muted: false,
    bitrateKbps: null,
    errorMessage: null,
    isBuffering: false,
  });
});

afterEach(() => {
  vi.useRealTimers();
});

describe('selectChannel', () => {
  it('sends a StationRequest built from the channel and credentials, never a URL', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    expect(playerPlay).toHaveBeenCalledExactlyOnceWith({
      baseUrl: creds.baseUrl,
      username: creds.username,
      password: creds.password,
      streamId: 42,
      stationId: '42',
    });
    expect(usePlayerStore.getState().currentChannel).toBe(channel);
  });

  it('applies the returned snapshot as the new status', async () => {
    vi.mocked(playerPlay).mockResolvedValueOnce(snapshot({ state: 'connecting', revision: 5 }));
    await usePlayerStore.getState().selectChannel(channel, creds);
    expect(usePlayerStore.getState().status).toBe('loading');
    expect(usePlayerStore.getState().isBuffering).toBe(true);
  });

  it('maps buffering and recovering to loading, and playing to playing', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);

    emitSnapshot(snapshot({ state: 'buffering' }));
    expect(usePlayerStore.getState().status).toBe('loading');
    expect(usePlayerStore.getState().isBuffering).toBe(true);

    emitSnapshot(snapshot({ state: 'playing' }));
    expect(usePlayerStore.getState().status).toBe('playing');
    expect(usePlayerStore.getState().isBuffering).toBe(false);
    expect(setMediaPlayback).toHaveBeenCalledWith(true);

    emitSnapshot(snapshot({ state: 'recovering' }));
    expect(usePlayerStore.getState().status).toBe('loading');
    expect(usePlayerStore.getState().isBuffering).toBe(true);
  });

  it('surfaces a load failure as an error state', async () => {
    vi.mocked(playerPlay).mockRejectedValueOnce(new Error('backend unavailable'));
    await expect(usePlayerStore.getState().selectChannel(channel, creds)).rejects.toThrow('backend unavailable');
    expect(usePlayerStore.getState().status).toBe('error');
    expect(usePlayerStore.getState().errorMessage).toBe('backend unavailable');
  });

  it('a failed snapshot surfaces its error message', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    emitSnapshot(snapshot({ state: 'failed', error: 'Upstream closed the connection' }));
    expect(usePlayerStore.getState().status).toBe('error');
    expect(usePlayerStore.getState().errorMessage).toBe('Upstream closed the connection');
  });

  it('a stale rejection does not clobber a newer channel selection', async () => {
    // Rust now routes ordinary startup failures through a snapshot instead of rejecting the
    // invoke (see commands.rs's `handle_startup_failure`), so a thrown rejection here models
    // an exceptional, slow-to-arrive failure racing a newer selection - not the normal retry
    // path. The stale rejection must never overwrite state the user has already moved past.
    let rejectFirst: (err: Error) => void = () => {};
    const firstCallPromise = new Promise<Snapshot>((_resolve, reject) => {
      rejectFirst = reject;
    });
    vi.mocked(playerPlay).mockImplementationOnce(() => firstCallPromise);

    const firstSelect = usePlayerStore.getState().selectChannel(channel, creds);
    firstSelect.catch(() => {}); // Its eventual rejection is asserted on below, not here.

    const otherChannel: XtreamChannel = { ...channel, stream_id: 99 };
    vi.mocked(playerPlay).mockResolvedValueOnce(
      snapshot({ state: 'playing', stationId: '99' }),
    );
    await usePlayerStore.getState().selectChannel(otherChannel, creds);
    expect(usePlayerStore.getState().status).toBe('playing');
    expect(usePlayerStore.getState().currentChannel?.stream_id).toBe(99);

    // The first (now stale) attempt's failure finally arrives.
    rejectFirst(new Error('stale device error'));
    await flushAsync();
    await expect(firstSelect).rejects.toThrow('stale device error');

    expect(usePlayerStore.getState().status).toBe('playing');
    expect(usePlayerStore.getState().currentChannel?.stream_id).toBe(99);
    expect(usePlayerStore.getState().errorMessage).toBeNull();
  });

  it('never builds a stream URL or retries - Rust owns connection attempts', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    for (const args of vi.mocked(playerPlay).mock.calls.map((c) => c[0])) {
      expect(args).not.toHaveProperty('url');
    }
    // A definitive failure never triggers a second play() call from the store -
    // Rust's own retry/fallback loop (if any) is invisible to the frontend.
    emitSnapshot(snapshot({ state: 'failed', error: 'permanent' }));
    await vi.advanceTimersByTimeAsync(20_000);
    expect(playerPlay).toHaveBeenCalledTimes(1);
  });
});

describe('stale snapshot ordering', () => {
  it('registers the snapshot listener before any command is issued', () => {
    expect(listenerRegisteredBeforeAnyCommand).toBe(true);
  });

  it('a stale snapshot (lower revision) does not overwrite newer state', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    // Revisions are a monotonically increasing singleton across the player's
    // whole lifetime (see playerStore.ts), so tests derive their revisions
    // relative to `nextRevision` rather than hardcoding absolute numbers -
    // this keeps them independent of how many snapshots earlier tests applied.
    const base = nextRevision;
    emitSnapshot(snapshot({ state: 'playing', revision: base + 10 }));
    expect(usePlayerStore.getState().status).toBe('playing');

    // Arrives late (e.g. a slow event delivery from an earlier revision).
    emitSnapshot(snapshot({ state: 'connecting', revision: base + 3 }));
    expect(usePlayerStore.getState().status).toBe('playing');
    expect(usePlayerStore.getState().isBuffering).toBe(false);
  });

  it('a snapshot with an equal revision does not overwrite newer state', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    const base = nextRevision;
    emitSnapshot(snapshot({ state: 'playing', revision: base + 7 }));
    emitSnapshot(snapshot({ state: 'failed', error: 'ignored', revision: base + 7 }));
    expect(usePlayerStore.getState().status).toBe('playing');
  });

  it('applies a snapshot with a strictly higher revision', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    const base = nextRevision;
    emitSnapshot(snapshot({ state: 'playing', revision: base + 7 }));
    emitSnapshot(snapshot({ state: 'stopped', revision: base + 8 }));
    expect(usePlayerStore.getState().status).toBe('stopped');
  });
});

describe('rapid station switches', () => {
  it('settles on the last channel selected', async () => {
    const other: XtreamChannel = { ...channel, stream_id: 99, name: 'The Pulse' };

    void usePlayerStore.getState().selectChannel(channel, creds);
    await usePlayerStore.getState().selectChannel(other, creds);

    expect(usePlayerStore.getState().currentChannel).toBe(other);
    expect(playerPlay).toHaveBeenLastCalledWith(expect.objectContaining({ streamId: 99, stationId: '99' }));
  });

  it('a late snapshot for the abandoned station does not clobber the new one once revisions are respected', async () => {
    void usePlayerStore.getState().selectChannel(channel, creds);
    const other: XtreamChannel = { ...channel, stream_id: 99, name: 'The Pulse' };
    await usePlayerStore.getState().selectChannel(other, creds);

    const base = nextRevision;
    emitSnapshot(snapshot({ state: 'playing', stationId: '99', revision: base + 20 }));
    expect(usePlayerStore.getState().status).toBe('playing');

    // A stale event for the old station 42, with a lower revision, must not win.
    emitSnapshot(snapshot({ state: 'connecting', stationId: '42', revision: base + 15 }));
    expect(usePlayerStore.getState().status).toBe('playing');
  });
});

describe('stop and play', () => {
  it('stop() halts playback and applies the returned snapshot', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    emitSnapshot(snapshot({ state: 'playing' }));

    await usePlayerStore.getState().stop();
    expect(playerStop).toHaveBeenCalledTimes(1);
    expect(usePlayerStore.getState().status).toBe('stopped');
    expect(setMediaPlayback).toHaveBeenLastCalledWith(false);
  });

  it('play() reconnects using the last selected channel and credentials', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    emitSnapshot(snapshot({ state: 'playing' }));
    await usePlayerStore.getState().stop();

    await usePlayerStore.getState().play();
    expect(playerPlay).toHaveBeenLastCalledWith(expect.objectContaining({ streamId: 42, stationId: '42' }));
  });

  it('play() is a no-op when no channel has ever been selected', async () => {
    await usePlayerStore.getState().play();
    expect(playerPlay).not.toHaveBeenCalled();
  });
});

describe('bitrate', () => {
  it('is fed directly from the snapshot', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    emitSnapshot(snapshot({ state: 'playing', bitrateKbps: 258 }));
    expect(usePlayerStore.getState().bitrateKbps).toBe(258);

    emitSnapshot(snapshot({ state: 'playing', bitrateKbps: null }));
    expect(usePlayerStore.getState().bitrateKbps).toBeNull();
  });
});

describe('volume and mute', () => {
  it('setVolume unmutes, routes to player_set_muted and player_set_volume, and echoes to the OS media widget', async () => {
    usePlayerStore.setState({ muted: true, currentChannel: channel });
    await usePlayerStore.getState().setVolume(50);

    expect(usePlayerStore.getState().muted).toBe(false);
    expect(playerSetMuted).toHaveBeenCalledWith(false);
    expect(playerSetVolume).toHaveBeenCalledWith(50);
    expect(setMediaVolume).toHaveBeenCalledWith(0.5);
  });

  it('routes to player_set_volume even with no channel selected', async () => {
    await usePlayerStore.getState().setVolume(30);
    expect(playerSetVolume).toHaveBeenCalledWith(30);
    expect(usePlayerStore.getState().volume).toBe(30);
  });

  it('debounces persisting the volume to settings', async () => {
    usePlayerStore.setState({ currentChannel: channel });
    await usePlayerStore.getState().setVolume(10);
    await usePlayerStore.getState().setVolume(20);
    expect(settingsUpdate).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(400);
    expect(settingsUpdate).toHaveBeenCalledExactlyOnceWith({ volume: 20 });
  });

  it('toggleMute flips the flag and routes to player_set_muted', async () => {
    usePlayerStore.setState({ currentChannel: channel });
    await usePlayerStore.getState().toggleMute();
    expect(usePlayerStore.getState().muted).toBe(true);
    expect(playerSetMuted).toHaveBeenCalledWith(true);

    await usePlayerStore.getState().toggleMute();
    expect(usePlayerStore.getState().muted).toBe(false);
    expect(playerSetMuted).toHaveBeenCalledWith(false);
  });
});

describe('OS media controls', () => {
  it('maps pause/toggle to stop (live radio has no pause)', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    emitSnapshot(snapshot({ state: 'playing' }));

    emitMediaControl('pause');
    await flushAsync();
    expect(playerStop).toHaveBeenCalledTimes(1);
    expect(usePlayerStore.getState().status).toBe('stopped');
  });

  it('maps play to reconnecting the current channel', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    emitSnapshot(snapshot({ state: 'playing' }));
    await usePlayerStore.getState().stop();

    emitMediaControl('play');
    await flushAsync();
    expect(playerPlay).toHaveBeenLastCalledWith(expect.objectContaining({ streamId: 42 }));
  });

  it('ignores media controls when no channel was ever selected', async () => {
    emitMediaControl('pause');
    await flushAsync();
    expect(playerStop).not.toHaveBeenCalled();
  });
});
