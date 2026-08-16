import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { XtreamChannel } from '../types/xtream';
import type { XtreamCredentials } from '../lib/xtream';
import { getLiveStreams } from '../lib/xtream';
import { useChannelStore } from './channelStore';

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
  useChannelStore.setState({ channels: [], status: 'idle', error: null });
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
