import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { StellarChannel } from '../types/stellarTunerLog';
import { invoke } from '@tauri-apps/api/core';
import { getChannels, getHistory, getNowPlaying } from './stellarTunerLog';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

// The request itself, its API-key header and its error shaping moved into Rust
// (src-tauri/src/stellar.rs) and are tested there. What stays here is the argument
// contract and the response reshaping the frontend still performs: the channels field
// arriving as either an array or a keyed record, and the SiriusXM art CDN downgrade,
// which has to happen frontend-side because the webview is what performs the failing
// TLS handshake for <img> loads.

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
  vi.mocked(invoke).mockReset();
});

describe('getNowPlaying', () => {
  it('passes the API key through when given one, and null when not', async () => {
    vi.mocked(invoke).mockResolvedValue({ stations: {} });
    await getNowPlaying('secret-key');
    expect(invoke).toHaveBeenLastCalledWith('stellar_now_playing', { apiKey: 'secret-key' });

    // Explicitly null rather than undefined: an omitted field would fail to deserialise
    // into Rust's Option<String> parameter.
    await getNowPlaying();
    expect(invoke).toHaveBeenLastCalledWith('stellar_now_playing', { apiKey: null });
  });

  it('propagates the error Rust produced', async () => {
    vi.mocked(invoke).mockRejectedValue(
      new Error('StellarTunerLog /nowplaying failed: HTTP 500'),
    );
    await expect(getNowPlaying()).rejects.toThrow('StellarTunerLog /nowplaying failed: HTTP 500');
  });
});

describe('getChannels', () => {
  it('accepts the channels field as an array', async () => {
    vi.mocked(invoke).mockResolvedValue({ channel_count: 1, channels: [stellarChannel()] });
    const channels = await getChannels();
    expect(channels).toHaveLength(1);
    expect(channels[0].id).toBe('chan');
  });

  it('accepts the channels field as a keyed record', async () => {
    vi.mocked(invoke).mockResolvedValue({
      channel_count: 2,
      channels: { a: stellarChannel({ id: 'a' }), b: stellarChannel({ id: 'b' }) },
    });
    const channels = await getChannels();
    expect(channels.map((c) => c.id)).toEqual(['a', 'b']);
  });

  it('downgrades only the broken SiriusXM art CDN host to http', async () => {
    vi.mocked(invoke).mockResolvedValue({
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
    });
    const [channel] = await getChannels();
    expect(channel.logos?.color_dark_square?.url).toBe(
      'http://pri.art.prod.streaming.siriusxm.com/logo.png',
    );
    expect(channel.logos?.white_square?.url).toBe('https://other.example.com/logo.png');
  });

  it('tolerates channels without logos', async () => {
    vi.mocked(invoke).mockResolvedValue({
      channel_count: 1,
      channels: [stellarChannel({ logos: undefined })],
    });
    await expect(getChannels()).resolves.toHaveLength(1);
  });
});

describe('getHistory', () => {
  it('passes the channel id and key, and unwraps the plays array', async () => {
    const plays = [{ played_at: '2026-07-17T00:00:00Z', artist: 'Artist', title: 'Title' }];
    vi.mocked(invoke).mockResolvedValue({ channel_id: 'chan', plays });

    await expect(getHistory('chan', 'secret-key')).resolves.toEqual(plays);
    expect(invoke).toHaveBeenLastCalledWith('stellar_history', {
      channelId: 'chan',
      apiKey: 'secret-key',
    });
  });

  it('throws with the status on HTTP failure', async () => {
    vi.mocked(invoke).mockRejectedValue(
      new Error('StellarTunerLog /history failed: HTTP 401'),
    );
    await expect(getHistory('chan', 'bad-key')).rejects.toThrow(
      'StellarTunerLog /history failed: HTTP 401',
    );
  });
});
