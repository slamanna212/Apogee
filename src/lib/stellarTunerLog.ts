import { invoke } from '@tauri-apps/api/core';
import type {
  StellarChannel,
  StellarChannelsResponse,
  StellarHistoryEntry,
  StellarHistoryResponse,
  StellarNowPlayingResponse,
} from '../types/stellarTunerLog';

/**
 * StellarTunerLog calls run in Rust (`src-tauri/src/stellar.rs`) so all application HTTP
 * shares one client. The endpoints' differing authentication is preserved deliberately:
 * /nowplaying and /channels are keyless, only /history requires an API key, so a missing
 * key costs you play history rather than now-playing metadata.
 */

/**
 * pri.art.prod.streaming.siriusxm.com's TLS cert doesn't cover its own hostname
 * (the Akamai edge falls back to a generic a248.e.akamai.net cert), so https
 * loads fail cert validation in the webview - http to the same host works fine.
 *
 * Applied here rather than in Rust because these URLs are consumed by <img> tags in the
 * webview, which is what performs the failing TLS handshake.
 */
function downgradeSiriusCdnUrl(url: string): string {
  return url.replace(/^https:\/\/(pri\.art\.prod\.streaming\.siriusxm\.com\/)/, 'http://$1');
}

/** No API key required for /nowplaying - only /history checks it. */
export function getNowPlaying(apiKey?: string): Promise<StellarNowPlayingResponse> {
  return invoke('stellar_now_playing', { apiKey: apiKey ?? null });
}

/** No API key required for /channels either - only /history checks it. */
export async function getChannels(): Promise<StellarChannel[]> {
  const data: StellarChannelsResponse = await invoke('stellar_channels');
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
  const data: StellarHistoryResponse = await invoke('stellar_history', { channelId, apiKey });
  return data.plays;
}
