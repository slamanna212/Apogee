import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { getLiveCategories, getLiveStreams, type XtreamCredentials } from './xtream';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

// URL construction, credential escaping and error-message redaction moved into Rust
// (src-tauri/src/xtream.rs) and are tested there against the real serialiser. What is
// left here is the argument contract, which is exactly what breaks silently if either
// side is renamed.

const creds: XtreamCredentials = {
  baseUrl: 'http://example.com:8080',
  username: 'alice',
  password: 'hunter2',
};

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});

describe('getLiveCategories', () => {
  it('invokes the Rust command with the credentials', async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await getLiveCategories(creds);
    expect(invoke).toHaveBeenCalledExactlyOnceWith('xtream_get_live_categories', { creds });
  });

  it('returns whatever Rust returns, unchanged', async () => {
    const categories = [{ category_id: '5', category_name: 'SiriusXM', parent_id: 0 }];
    vi.mocked(invoke).mockResolvedValue(categories);
    await expect(getLiveCategories(creds)).resolves.toEqual(categories);
  });

  it('propagates the error Rust produced rather than rewrapping it', async () => {
    // Rust already redacts; rewrapping here would risk re-adding a URL.
    vi.mocked(invoke).mockRejectedValue(new Error('get_live_categories failed: HTTP 403'));
    await expect(getLiveCategories(creds)).rejects.toThrow('get_live_categories failed: HTTP 403');
  });

  it('never builds a URL itself', async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await getLiveCategories(creds);
    const [, args] = vi.mocked(invoke).mock.calls[0];
    expect(JSON.stringify(args)).not.toContain('player_api.php');
  });
});

describe('getLiveStreams', () => {
  it('passes the category id under the name the command expects', async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await getLiveStreams(creds, '12');
    expect(invoke).toHaveBeenCalledExactlyOnceWith('xtream_get_live_streams', {
      creds,
      categoryId: '12',
    });
  });
});
