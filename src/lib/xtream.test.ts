import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fetchWithTimeout } from './fetchWithTimeout';
import { buildStreamUrl, getLiveCategories, getLiveStreams, type XtreamCredentials } from './xtream';

vi.mock('./fetchWithTimeout', () => ({ fetchWithTimeout: vi.fn() }));

const creds: XtreamCredentials = {
  baseUrl: 'http://example.com:8080',
  username: 'alice',
  password: 'hunter2',
};

function okResponse(body: unknown): Response {
  return { ok: true, status: 200, json: async () => body } as Response;
}

beforeEach(() => {
  vi.mocked(fetchWithTimeout).mockReset();
});

describe('getLiveCategories', () => {
  it('requests player_api.php with credentials and the action', async () => {
    vi.mocked(fetchWithTimeout).mockResolvedValue(okResponse([]));
    await getLiveCategories(creds);

    const url = new URL(vi.mocked(fetchWithTimeout).mock.calls[0][0]);
    expect(url.pathname).toBe('/player_api.php');
    expect(url.searchParams.get('username')).toBe('alice');
    expect(url.searchParams.get('password')).toBe('hunter2');
    expect(url.searchParams.get('action')).toBe('get_live_categories');
  });

  it('returns the parsed JSON body', async () => {
    const categories = [{ category_id: '5', category_name: 'SiriusXM', parent_id: 0 }];
    vi.mocked(fetchWithTimeout).mockResolvedValue(okResponse(categories));
    await expect(getLiveCategories(creds)).resolves.toEqual(categories);
  });

  it('surfaces HTTP failures with the status code', async () => {
    vi.mocked(fetchWithTimeout).mockResolvedValue({ ok: false, status: 403 } as Response);
    await expect(getLiveCategories(creds)).rejects.toThrow('get_live_categories failed: HTTP 403');
  });

  it('never leaks the credential-bearing URL from a network error', async () => {
    vi.mocked(fetchWithTimeout).mockRejectedValue(
      new Error('fetch failed: http://example.com:8080/player_api.php?username=alice&password=hunter2'),
    );
    const err = await getLiveCategories(creds).catch((e: Error) => e);
    expect(err).toBeInstanceOf(Error);
    expect((err as Error).message).toBe('get_live_categories failed: could not reach the Xtream server');
    expect((err as Error).message).not.toContain('hunter2');
  });
});

describe('getLiveStreams', () => {
  it('passes the category_id parameter', async () => {
    vi.mocked(fetchWithTimeout).mockResolvedValue(okResponse([]));
    await getLiveStreams(creds, '12');

    const url = new URL(vi.mocked(fetchWithTimeout).mock.calls[0][0]);
    expect(url.searchParams.get('action')).toBe('get_live_streams');
    expect(url.searchParams.get('category_id')).toBe('12');
  });

  it('normalizes network errors without the URL', async () => {
    vi.mocked(fetchWithTimeout).mockRejectedValue(new TypeError('Load failed'));
    await expect(getLiveStreams(creds, '12')).rejects.toThrow(
      'get_live_streams failed: could not reach the Xtream server',
    );
  });
});

describe('buildStreamUrl', () => {
  it('builds the live stream URL from credentials, stream id, and extension', () => {
    expect(buildStreamUrl(creds, 42, '.ts')).toBe('http://example.com:8080/live/alice/hunter2/42.ts');
    expect(buildStreamUrl(creds, 42, '.m3u8')).toBe('http://example.com:8080/live/alice/hunter2/42.m3u8');
  });

  it('strips trailing slashes from the base URL', () => {
    const slashed = { ...creds, baseUrl: 'http://example.com:8080///' };
    expect(buildStreamUrl(slashed, 7, '.ts')).toBe('http://example.com:8080/live/alice/hunter2/7.ts');
  });
});
