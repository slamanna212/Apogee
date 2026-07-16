import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarStation } from '../types/stellarTunerLog';

export const MATCH_THRESHOLD = 0.85;

export function normalizeChannelName(name: string): string {
  let normalized = name.trim().toLowerCase();
  normalized = normalized.replace(/^radio:\s*/, '');
  normalized = normalized.replace(/^the\s+/, '');
  normalized = normalized.replace(/\s+(hd|radio)$/i, '');
  normalized = normalized.replace(/[^a-z0-9]+/g, '');
  return normalized;
}

function levenshtein(a: string, b: string): number {
  const dp: number[][] = Array.from({ length: a.length + 1 }, () => new Array(b.length + 1).fill(0));
  for (let i = 0; i <= a.length; i++) dp[i][0] = i;
  for (let j = 0; j <= b.length; j++) dp[0][j] = j;
  for (let i = 1; i <= a.length; i++) {
    for (let j = 1; j <= b.length; j++) {
      dp[i][j] =
        a[i - 1] === b[j - 1]
          ? dp[i - 1][j - 1]
          : 1 + Math.min(dp[i - 1][j - 1], dp[i - 1][j], dp[i][j - 1]);
    }
  }
  return dp[a.length][b.length];
}

export function nameSimilarity(a: string, b: string): number {
  const maxLen = Math.max(a.length, b.length);
  if (maxLen === 0) return 1;
  return 1 - levenshtein(a, b) / maxLen;
}

export function findBestMatch<T>(
  xtreamName: string,
  items: T[],
  nameOf: (item: T) => string,
): T | null {
  const target = normalizeChannelName(xtreamName);
  let best: T | null = null;
  let bestScore = 0;
  for (const item of items) {
    const score = nameSimilarity(target, normalizeChannelName(nameOf(item)));
    if (score > bestScore) {
      bestScore = score;
      best = item;
    }
  }
  return bestScore >= MATCH_THRESHOLD ? best : null;
}

export function findBestStationMatch(
  xtreamName: string,
  stations: StellarStation[],
): StellarStation | null {
  return findBestMatch(xtreamName, stations, (station) => station.name);
}

/**
 * Builds the channel -> live-station now-playing map for one poll tick.
 *
 * The channel/station name match is stable (a channel doesn't change which
 * station it corresponds to between polls) while only the song/artist inside
 * a station changes - so `stationIdCache` remembers each channel's matched
 * `StellarStation.id` and this only re-runs the O(len^2) fuzzy match for
 * channels that aren't cached yet (first tick, or a channel whose station
 * dropped out of the current response), instead of re-matching every channel
 * against every station on every tick.
 */
export function buildNowPlayingMap(
  channels: XtreamChannel[],
  stations: StellarStation[],
  stationIdCache: Map<number, string>,
): Map<number, StellarStation> {
  const stationsById = new Map(stations.map((station) => [station.id, station]));
  const map = new Map<number, StellarStation>();
  for (const channel of channels) {
    const cachedId = stationIdCache.get(channel.stream_id);
    let station = cachedId ? stationsById.get(cachedId) : undefined;
    if (!station) {
      const match = findBestStationMatch(channel.name, stations);
      if (!match) continue;
      station = match;
      stationIdCache.set(channel.stream_id, match.id);
    }
    map.set(channel.stream_id, station);
  }
  return map;
}

/**
 * Shallow content comparison for two now-playing maps (by the fields
 * actually rendered), so a poll tick that returned identical data can keep
 * the previous `Map` reference instead of forcing a re-render everywhere
 * `nowPlaying` is subscribed to.
 */
export function nowPlayingMapsEqual(
  a: Map<number, StellarStation>,
  b: Map<number, StellarStation>,
): boolean {
  if (a.size !== b.size) return false;
  for (const [streamId, station] of b) {
    const prev = a.get(streamId);
    if (
      !prev ||
      prev.id !== station.id ||
      prev.title !== station.title ||
      prev.artist !== station.artist ||
      prev.album !== station.album ||
      prev.cut_type !== station.cut_type ||
      prev.artwork_url !== station.artwork_url
    ) {
      return false;
    }
  }
  return true;
}

export function buildChannelMetadataMap(
  channels: XtreamChannel[],
  stellarChannels: StellarChannel[],
): Map<number, StellarChannel> {
  const map = new Map<number, StellarChannel>();
  for (const channel of channels) {
    const match = findBestMatch(channel.name, stellarChannels, (c) => c.marketing_name || c.name);
    if (match) {
      map.set(channel.stream_id, match);
    }
  }
  return map;
}
