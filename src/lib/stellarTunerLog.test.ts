import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { StellarChannel } from '../types/stellarTunerLog';
import { fetchWithTimeout } from './fetchWithTimeout';
import { getChannels, getHistory, getNowPlaying } from './stellarTunerLog';

vi.mock('./fetchWithTimeout', () => ({ fetchWithTimeout: vi.fn() }));

function okResponse(body: unknown): Response {
  return { ok: true, status: 200, json: async () => body } as Response;
}

function stellarChannel(overrides: Partial<StellarChannel> = {}): StellarChannel {
  return {
    id: 'chan',
    name: 'Octane',
    marketing_name: 'Octane',
    channel_number: 37,
    categories: [],
    ...overrides,
  };
}

beforeEach(() => {
  vi.mocked(fetchWithTimeout).mockReset();
});

describe('getNowPlaying', () => {
  it('sends the API key header only when one is provided', async () => {
    vi.mocked(fetchWithTimeout).mockResolvedValue(okResponse({ stations: {} }));
    await getNowPlaying('secret-key');
    expect(fetchWithTimeout).toHaveBeenLastCalledWith(
      'https://api.stellartunerlog.com/v1/nowplaying',
      { headers: { 'X-API-Key': 'secret-key' } },
    );

    await getNowPlaying();
    expect(fetchWithTimeout).toHaveBeenLastCalledWith(
      'https://api.stellartunerlog.com/v1/nowplaying',
      { headers: undefined },
    );
  });

  it('throws with the status on HTTP failure', async () => {
    vi.mocked(fetchWithTimeout).mockResolvedValue({ ok: false, status: 500 } as Response);
    await expect(getNowPlaying()).rejects.toThrow('StellarTunerLog /nowplaying failed: HTTP 500');
  });
});

describe('getChannels', () => {
  it('accepts the channels field as an array', async () => {
    vi.mocked(fetchWithTimeout).mockResolvedValue(
      okResponse({ channel_count: 1, channels: [stellarChannel()] }),
    );
    const channels = await getChannels();
    expect(channels).toHaveLength(1);
    expect(channels[0].id).toBe('chan');
  });

  it('accepts the channels field as a keyed record', async () => {
    vi.mocked(fetchWithTimeout).mockResolvedValue(
      okResponse({
        channel_count: 2,
        channels: { a: stellarChannel({ id: 'a' }), b: stellarChannel({ id: 'b' }) },
      }),
    );
    const channels = await getChannels();
    expect(channels.map((c) => c.id)).toEqual(['a', 'b']);
  });

  it('downgrades only the broken SiriusXM art CDN host to http', async () => {
    vi.mocked(fetchWithTimeout).mockResolvedValue(
      okResponse({
        channel_count: 1,
        channels: [
          stellarChannel({
            logos: {
              color_dark_square: {
                url: 'https://pri.art.prod.streaming.siriusxm.com/logo.png',
                width: 300,
                height: 300,
              },
              white_square: {
                url: 'https://other.example.com/logo.png',
                width: 300,
                height: 300,
              },
            },
          }),
        ],
      }),
    );
    const [channel] = await getChannels();
    expect(channel.logos?.color_dark_square?.url).toBe(
      'http://pri.art.prod.streaming.siriusxm.com/logo.png',
    );
    expect(channel.logos?.white_square?.url).toBe('https://other.example.com/logo.png');
  });

  it('tolerates channels without logos', async () => {
    vi.mocked(fetchWithTimeout).mockResolvedValue(
      okResponse({ channel_count: 1, channels: [stellarChannel({ logos: undefined })] }),
    );
    await expect(getChannels()).resolves.toHaveLength(1);
  });
});

describe('getHistory', () => {
  it('requires the API key header and unwraps the plays array', async () => {
    const plays = [{ played_at: '2026-07-17T00:00:00Z', artist: 'Artist', title: 'Title' }];
    vi.mocked(fetchWithTimeout).mockResolvedValue(okResponse({ channel_id: 'chan', plays }));

    await expect(getHistory('chan', 'secret-key')).resolves.toEqual(plays);
    expect(fetchWithTimeout).toHaveBeenLastCalledWith(
      'https://api.stellartunerlog.com/v1/history/chan',
      { headers: { 'X-API-Key': 'secret-key' } },
    );
  });

  it('throws with the status on HTTP failure', async () => {
    vi.mocked(fetchWithTimeout).mockResolvedValue({ ok: false, status: 401 } as Response);
    await expect(getHistory('chan', 'bad-key')).rejects.toThrow(
      'StellarTunerLog /history failed: HTTP 401',
    );
  });
});
