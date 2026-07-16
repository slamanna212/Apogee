import { fetchWithTimeout } from './fetchWithTimeout';
import type { XtreamCategory, XtreamChannel } from '../types/xtream';

export interface XtreamCredentials {
  baseUrl: string;
  username: string;
  password: string;
}

function playerApiUrl(creds: XtreamCredentials, params: Record<string, string>) {
  const url = new URL('/player_api.php', creds.baseUrl);
  url.searchParams.set('username', creds.username);
  url.searchParams.set('password', creds.password);
  for (const [key, value] of Object.entries(params)) {
    url.searchParams.set(key, value);
  }
  return url.toString();
}

/**
 * Runs a player_api.php request and normalizes any failure into an Error
 * with a message that never contains the request URL - it embeds the
 * Xtream username/password (see playerApiUrl), and a raw fetch/network
 * error's message can include the URL it was trying to reach. Callers
 * surface these messages directly in UI-visible store state, so a leak
 * here would be user-visible, not just log-visible.
 */
async function xtreamRequest(action: string, creds: XtreamCredentials, params: Record<string, string> = {}) {
  try {
    const res = await fetchWithTimeout(playerApiUrl(creds, { action, ...params }));
    if (!res.ok) {
      throw new Error(`${action} failed: HTTP ${res.status}`);
    }
    return res;
  } catch (err) {
    if (err instanceof Error && err.message.startsWith(`${action} failed: HTTP`)) throw err;
    throw new Error(`${action} failed: could not reach the Xtream server`);
  }
}

export async function getLiveCategories(creds: XtreamCredentials): Promise<XtreamCategory[]> {
  const res = await xtreamRequest('get_live_categories', creds);
  return res.json();
}

export async function getLiveStreams(
  creds: XtreamCredentials,
  categoryId: string,
): Promise<XtreamChannel[]> {
  const res = await xtreamRequest('get_live_streams', creds, { category_id: categoryId });
  return res.json();
}

export function buildStreamUrl(
  creds: XtreamCredentials,
  streamId: number,
  extension: string,
): string {
  const base = creds.baseUrl.replace(/\/+$/, '');
  return `${base}/live/${creds.username}/${creds.password}/${streamId}${extension}`;
}
