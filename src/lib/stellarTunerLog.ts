import { fetchWithTimeout as fetch } from './fetchWithTimeout';
import type {
  StellarChannel,
  StellarChannelsResponse,
  StellarHistoryEntry,
  StellarHistoryResponse,
  StellarNowPlayingResponse,
} from '../types/stellarTunerLog';

const NOWPLAYING_URL = 'https://api.stellartunerlog.com/v1/nowplaying';
const CHANNELS_URL = 'https://api.stellartunerlog.com/v1/channels';
const historyUrl = (channelId: string) => `https://api.stellartunerlog.com/v1/history/${channelId}`;

/**
 * pri.art.prod.streaming.siriusxm.com's TLS cert doesn't cover its own hostname
 * (the Akamai edge falls back to a generic a248.e.akamai.net cert), so https
 * loads fail cert validation in the webview - http to the same host works fine.
 */
function downgradeSiriusCdnUrl(url: string): string {
  return url.replace(/^https:\/\/(pri\.art\.prod\.streaming\.siriusxm\.com\/)/, 'http://$1');
}

/** No API key required for /nowplaying - only /history checks it. */
export async function getNowPlaying(apiKey?: string): Promise<StellarNowPlayingResponse> {
  const res = await fetch(NOWPLAYING_URL, {
    headers: apiKey ? { 'X-API-Key': apiKey } : undefined,
  });
  if (!res.ok) {
    throw new Error(`StellarTunerLog /nowplaying failed: HTTP ${res.status}`);
  }
  return res.json();
}

/** No API key required for /channels either - only /history checks it. */
export async function getChannels(): Promise<StellarChannel[]> {
  const res = await fetch(CHANNELS_URL);
  if (!res.ok) {
    throw new Error(`StellarTunerLog /channels failed: HTTP ${res.status}`);
  }
  const data: StellarChannelsResponse = await res.json();
  const channels = Array.isArray(data.channels) ? data.channels : Object.values(data.channels);
  for (const channel of channels) {
    if (!channel.logos) continue;
    for (const key of Object.keys(channel.logos)) {
      const logo = channel.logos[key];
      if (logo) logo.url = downgradeSiriusCdnUrl(logo.url);
    }
  }
  return channels;
}

/**
 * Returns the last 24 hours of play history in one call - the endpoint takes
 * no limit/page params at all, so any pagination is handled client-side by
 * callers.
 */
export async function getHistory(channelId: string, apiKey: string): Promise<StellarHistoryEntry[]> {
  const res = await fetch(historyUrl(channelId), {
    headers: { 'X-API-Key': apiKey },
  });
  if (!res.ok) {
    throw new Error(`StellarTunerLog /history failed: HTTP ${res.status}`);
  }
  const data: StellarHistoryResponse = await res.json();
  return data.plays;
}
