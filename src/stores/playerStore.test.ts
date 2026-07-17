import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { XtreamChannel } from '../types/xtream';
import type { XtreamCredentials } from '../lib/xtream';
import {
  getStderrTail,
  loadUrl,
  onMpvEvent,
  setMute as mpvSetMute,
  setVolume as mpvSetVolume,
  stopPlayback,
} from '../lib/mpvClient';
import { onMediaControlEvent, setMediaPlayback, setMediaVolume } from '../lib/mediaSession';
import { usePlayerStore } from './playerStore';

const settingsUpdate = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/plugin-log', () => ({
  debug: vi.fn(),
  info: vi.fn(),
  warn: vi.fn(),
  error: vi.fn(),
}));
vi.mock('../lib/mpvClient', () => ({
  GET_PROPERTY_REQUEST_ID: 777,
  getProperty: vi.fn().mockResolvedValue(undefined),
  getStderrTail: vi.fn().mockResolvedValue(''),
  loadUrl: vi.fn().mockResolvedValue(undefined),
  onMpvEvent: vi.fn().mockResolvedValue(() => {}),
  stopPlayback: vi.fn().mockResolvedValue(undefined),
  setProperty: vi.fn().mockResolvedValue(undefined),
  setVolume: vi.fn().mockResolvedValue(undefined),
  setMute: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('../lib/mediaSession', () => ({
  onMediaControlEvent: vi.fn(),
  setMediaPlayback: vi.fn().mockResolvedValue(undefined),
  setMediaVolume: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('./settingsStore', () => ({
  useSettingsStore: { getState: () => ({ update: settingsUpdate, settings: { audioDevice: null } }) },
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

const TS_URL = 'http://example.com:8080/live/user/pass/42.ts';
const M3U8_URL = 'http://example.com:8080/live/user/pass/42.m3u8';

// Registered once - the store guards against double registration, so capture
// the callbacks a single time and reuse them across tests.
usePlayerStore.getState().initEventListener();
const emitMpv = vi.mocked(onMpvEvent).mock.calls[0][0];
const emitMediaControl = vi.mocked(onMediaControlEvent).mock.calls[0][0];

async function flushAsync() {
  await vi.advanceTimersByTimeAsync(0);
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllTimers();
  vi.clearAllMocks();
  vi.mocked(getStderrTail).mockResolvedValue('');
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
  it('connects with the .ts extension first and applies the current volume', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    expect(loadUrl).toHaveBeenCalledExactlyOnceWith(TS_URL);
    expect(mpvSetVolume).toHaveBeenCalledWith(80);
    expect(mpvSetMute).not.toHaveBeenCalled();
    expect(usePlayerStore.getState().status).toBe('loading');
    expect(usePlayerStore.getState().currentChannel).toBe(channel);
  });

  it('only flips to playing once mpv reports playback-restart', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    expect(usePlayerStore.getState().status).toBe('loading');

    emitMpv({ event: 'playback-restart' });
    expect(usePlayerStore.getState().status).toBe('playing');
    expect(setMediaPlayback).toHaveBeenCalledWith(true);
  });

  it('surfaces a load failure as an error state', async () => {
    vi.mocked(loadUrl).mockRejectedValueOnce(new Error('mpv not running'));
    await expect(usePlayerStore.getState().selectChannel(channel, creds)).rejects.toThrow('mpv not running');
    expect(usePlayerStore.getState().status).toBe('error');
    expect(usePlayerStore.getState().errorMessage).toBe('mpv not running');
  });
});

describe('connect retries', () => {
  it('retries with the alternate .m3u8 extension after mpv reports an error', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    emitMpv({ event: 'end-file', reason: 'error' });
    await vi.advanceTimersByTimeAsync(1500);

    expect(loadUrl).toHaveBeenCalledTimes(2);
    expect(loadUrl).toHaveBeenLastCalledWith(M3U8_URL);
    expect(usePlayerStore.getState().status).toBe('loading');
  });

  it('treats a stalled connection (no playback-restart) as a failed attempt', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    await vi.advanceTimersByTimeAsync(20_000); // connect timeout
    await vi.advanceTimersByTimeAsync(1500); // retry delay

    expect(loadUrl).toHaveBeenCalledTimes(2);
    expect(loadUrl).toHaveBeenLastCalledWith(M3U8_URL);
  });

  it('alternates extensions and gives up after 4 attempts with mpv stderr attached', async () => {
    vi.mocked(getStderrTail).mockResolvedValue('Failed to open stream');
    await usePlayerStore.getState().selectChannel(channel, creds);
    for (let attempt = 0; attempt < 4; attempt++) {
      emitMpv({ event: 'end-file', reason: 'error' });
      await vi.advanceTimersByTimeAsync(1500);
    }

    expect(loadUrl).toHaveBeenCalledTimes(4);
    expect(vi.mocked(loadUrl).mock.calls.map(([url]) => url)).toEqual([
      TS_URL,
      M3U8_URL,
      TS_URL,
      M3U8_URL,
    ]);
    await vi.waitFor(() => expect(usePlayerStore.getState().status).toBe('error'));
    expect(usePlayerStore.getState().errorMessage).toBe(
      'Failed to connect after 4 attempts - Failed to open stream',
    );
  });

  it('retries only the currently selected channel after switching mid-connect', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    const other: XtreamChannel = { ...channel, stream_id: 99, name: 'The Pulse' };
    await usePlayerStore.getState().selectChannel(other, creds);

    // A failure now belongs to the new selection - the abandoned stream 42
    // must never be retried.
    emitMpv({ event: 'end-file', reason: 'error' });
    await vi.advanceTimersByTimeAsync(1500);
    expect(loadUrl).toHaveBeenLastCalledWith('http://example.com:8080/live/user/pass/99.m3u8');
    const urls = vi.mocked(loadUrl).mock.calls.map(([url]) => url);
    expect(urls.filter((url) => url.includes('/42.'))).toEqual([TS_URL]);
  });
});

describe('stop and play', () => {
  it('stop() halts playback and play() reconnects to the last URL', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    emitMpv({ event: 'playback-restart' });

    await usePlayerStore.getState().stop();
    expect(stopPlayback).toHaveBeenCalledTimes(1);
    expect(usePlayerStore.getState().status).toBe('stopped');
    expect(setMediaPlayback).toHaveBeenLastCalledWith(false);

    await usePlayerStore.getState().play();
    expect(loadUrl).toHaveBeenLastCalledWith(TS_URL);
    expect(usePlayerStore.getState().status).toBe('loading');
  });

  it('reconnects to the same URL when the mpv IPC connection drops mid-playback', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    emitMpv({ event: 'playback-restart' });
    expect(loadUrl).toHaveBeenCalledTimes(1);

    emitMpv({ event: 'apogee-ipc-closed' });
    await flushAsync();
    expect(loadUrl).toHaveBeenCalledTimes(2);
    expect(loadUrl).toHaveBeenLastCalledWith(TS_URL);
    expect(usePlayerStore.getState().status).toBe('loading');
  });

  it('does not reconnect on IPC loss when playback was already stopped', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    emitMpv({ event: 'playback-restart' });
    await usePlayerStore.getState().stop();
    const calls = vi.mocked(loadUrl).mock.calls.length;

    emitMpv({ event: 'apogee-ipc-closed' });
    await flushAsync();
    expect(loadUrl).toHaveBeenCalledTimes(calls);
  });
});

describe('bitrate events', () => {
  it('derives kbps from audio-bitrate property changes and clears on null', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    emitMpv({ event: 'property-change', name: 'audio-bitrate', data: 128_000 });
    expect(usePlayerStore.getState().bitrateKbps).toBe(128);

    emitMpv({ event: 'property-change', name: 'audio-bitrate', data: null });
    expect(usePlayerStore.getState().bitrateKbps).toBeNull();
  });

  it('accepts the fallback packet-audio-bitrate reply by request id', () => {
    emitMpv({ request_id: 777, data: 96_000 });
    expect(usePlayerStore.getState().bitrateKbps).toBe(96);
  });

  it('tracks mid-playback buffering via core-idle', () => {
    emitMpv({ event: 'property-change', name: 'core-idle', data: true });
    expect(usePlayerStore.getState().isBuffering).toBe(true);
    emitMpv({ event: 'property-change', name: 'core-idle', data: false });
    expect(usePlayerStore.getState().isBuffering).toBe(false);
  });
});

describe('volume and mute', () => {
  it('setVolume unmutes, applies to mpv, and echoes to the OS media widget', async () => {
    usePlayerStore.setState({ muted: true, currentChannel: channel });
    await usePlayerStore.getState().setVolume(50);

    expect(usePlayerStore.getState().muted).toBe(false);
    expect(mpvSetMute).toHaveBeenCalledWith(false);
    expect(mpvSetVolume).toHaveBeenCalledWith(50);
    expect(setMediaVolume).toHaveBeenCalledWith(0.5);
  });

  it('debounces persisting the volume to settings', async () => {
    usePlayerStore.setState({ currentChannel: channel });
    await usePlayerStore.getState().setVolume(10);
    await usePlayerStore.getState().setVolume(20);
    expect(settingsUpdate).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(400);
    expect(settingsUpdate).toHaveBeenCalledExactlyOnceWith({ volume: 20 });
  });

  it('does not talk to mpv when no channel is selected', async () => {
    await usePlayerStore.getState().setVolume(30);
    expect(mpvSetVolume).not.toHaveBeenCalled();
    expect(usePlayerStore.getState().volume).toBe(30);
  });

  it('toggleMute flips the flag and forwards it to mpv', async () => {
    usePlayerStore.setState({ currentChannel: channel });
    await usePlayerStore.getState().toggleMute();
    expect(usePlayerStore.getState().muted).toBe(true);
    expect(mpvSetMute).toHaveBeenCalledWith(true);

    await usePlayerStore.getState().toggleMute();
    expect(usePlayerStore.getState().muted).toBe(false);
    expect(mpvSetMute).toHaveBeenCalledWith(false);
  });
});

describe('OS media controls', () => {
  it('maps pause/toggle to stop (live radio has no pause)', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    emitMpv({ event: 'playback-restart' });

    emitMediaControl('pause');
    await flushAsync();
    expect(stopPlayback).toHaveBeenCalledTimes(1);
    expect(usePlayerStore.getState().status).toBe('stopped');
  });

  it('maps play to reconnecting the current channel', async () => {
    await usePlayerStore.getState().selectChannel(channel, creds);
    emitMpv({ event: 'playback-restart' });
    await usePlayerStore.getState().stop();

    emitMediaControl('play');
    await flushAsync();
    expect(loadUrl).toHaveBeenLastCalledWith(TS_URL);
    expect(usePlayerStore.getState().status).toBe('loading');
  });

  it('ignores media controls when no channel was ever selected', async () => {
    emitMediaControl('pause');
    await flushAsync();
    expect(stopPlayback).not.toHaveBeenCalled();
  });
});
