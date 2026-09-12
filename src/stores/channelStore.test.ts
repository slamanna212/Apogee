import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { XtreamChannel } from '../types/xtream';
import type { XtreamCredentials } from '../lib/xtream';
import { getLiveStreams } from '../lib/xtream';
import { nextPollDelayMs, useChannelStore } from './channelStore';
import { getNowPlaying } from '../lib/stellarTunerLog';
import type { StellarNowPlayingResponse, StellarStation } from '../types/stellarTunerLog';

vi.mock('@tauri-apps/plugin-log', () => ({
  warn: vi.fn(async () => {}),
  debug: vi.fn(async () => {}),
}));

vi.mock('../lib/xtream', () => ({
  getLiveStreams: vi.fn(),
}));
vi.mock('../lib/stellarTunerLog', () => ({
  getChannels: vi.fn(),
  getNowPlaying: vi.fn(),
}));
vi.mock('@tauri-apps/plugin-store', () => ({
  load: vi.fn(async () => ({
    get: vi.fn(async () => undefined),
    set: vi.fn(async () => {}),
    save: vi.fn(async () => {}),
  })),
}));

const creds: XtreamCredentials = {
  baseUrl: 'http://example.com:8080',
  username: 'alice',
  password: 'hunter2',
};

function channel(streamId: number, num: number): XtreamChannel {
  return { stream_id: streamId, num, name: `Channel ${streamId}` } as XtreamChannel;
}

beforeEach(() => {
  vi.mocked(getLiveStreams).mockReset();
  vi.mocked(getNowPlaying).mockReset();
  useChannelStore.setState({ channels: [], status: 'idle', error: null, nowPlaying: new Map(), pollFailureCount: 0 });
});

describe('now-playing polling', () => {
  function response(title: string): StellarNowPlayingResponse {
    const station: StellarStation = {
      id: 'poll-test', name: 'Channel 1', channel_number: 1, artist: 'Artist',
      title, album: 'Album', cut_type: 'Song', artwork_url: 'cover.png', itunes_id: '',
    };
    return { stations: { station }, station_count: 1, updated_utc: '', poll_interval_seconds: 10 };
  }

  it('accounts for request duration and bounds recovery delays', () => {
    expect(nextPollDelayMs(0, 4000)).toBe(6000);
    expect(nextPollDelayMs(0, 12000)).toBe(1000);
    expect([1, 2, 3, 10].map((failures) => nextPollDelayMs(failures))).toEqual([10000, 20000, 30000, 30000]);
  });

  it('keeps metadata on failure and resets backoff after recovery', async () => {
    useChannelStore.setState({ channels: [channel(1, 1)] });
    vi.mocked(getNowPlaying).mockResolvedValueOnce(response('Song'));
    await useChannelStore.getState().pollNowPlaying();
    const previous = useChannelStore.getState().nowPlaying;
    vi.mocked(getNowPlaying).mockRejectedValueOnce(new Error('offline'));
    await useChannelStore.getState().pollNowPlaying();
    expect(useChannelStore.getState().nowPlaying).toBe(previous);
    expect(useChannelStore.getState().pollFailureCount).toBe(1);
    vi.mocked(getNowPlaying).mockResolvedValueOnce(response('Next'));
    await useChannelStore.getState().pollNowPlaying();
    expect(useChannelStore.getState().pollFailureCount).toBe(0);
    expect(useChannelStore.getState().nowPlaying.get(1)?.title).toBe('Next');
  });

  it('ignores an older request completing after a newer poll', async () => {
    useChannelStore.setState({ channels: [channel(1, 1)] });
    let resolve!: (value: StellarNowPlayingResponse) => void;
    vi.mocked(getNowPlaying).mockReturnValueOnce(new Promise((done) => { resolve = done; }));
    const oldPoll = useChannelStore.getState().pollNowPlaying();
    vi.mocked(getNowPlaying).mockResolvedValueOnce(response('New'));
    await useChannelStore.getState().pollNowPlaying();
    resolve(response('Old'));
    await oldPoll;
    expect(useChannelStore.getState().nowPlaying.get(1)?.title).toBe('New');
  });
});

describe('channelStore.fetchChannels', () => {
  it('calls getLiveStreams once per category id', async () => {
    vi.mocked(getLiveStreams).mockResolvedValue([]);
    await useChannelStore.getState().fetchChannels(creds, ['1', '2']);

    expect(getLiveStreams).toHaveBeenCalledTimes(2);
    expect(getLiveStreams).toHaveBeenCalledWith(creds, '1');
    expect(getLiveStreams).toHaveBeenCalledWith(creds, '2');
  });

  it('merges channels from multiple categories, sorted by num', async () => {
    vi.mocked(getLiveStreams).mockImplementation(async (_creds, categoryId) => {
      if (categoryId === '1') return [channel(10, 2)];
      return [channel(20, 1)];
    });

    await useChannelStore.getState().fetchChannels(creds, ['1', '2']);

    expect(useChannelStore.getState().status).toBe('loaded');
    expect(useChannelStore.getState().channels.map((c) => c.stream_id)).toEqual([20, 10]);
  });

  it('dedupes a stream_id that appears in more than one category', async () => {
    vi.mocked(getLiveStreams).mockImplementation(async (_creds, categoryId) => {
      if (categoryId === '1') return [channel(10, 1)];
      return [channel(10, 1)];
    });

    await useChannelStore.getState().fetchChannels(creds, ['1', '2']);

    expect(useChannelStore.getState().channels).toHaveLength(1);
  });

  it('sets status to error when a category fetch fails', async () => {
    vi.mocked(getLiveStreams).mockRejectedValue(new Error('boom'));

    await useChannelStore.getState().fetchChannels(creds, ['1']);

    expect(useChannelStore.getState().status).toBe('error');
    expect(useChannelStore.getState().error).toBe('boom');
  });
});
