import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel } from '../types/stellarTunerLog';
import { parseChannelName, stripTrailingNoise } from './channelMatcher';

/**
 * What a channel should be called when StellarTunerLog metadata is missing:
 * the provider's name with its channel-number prefix and any quality/locale
 * noise removed ("43 Rock The Bells Radio HD" -> "Rock The Bells Radio"),
 * rather than the raw string.
 */
export function cleanChannelName(rawName: string): string {
  const cleaned = stripTrailingNoise(parseChannelName(rawName).name);
  return cleaned.length > 0 ? cleaned : rawName;
}

export function channelDisplayName(channel: XtreamChannel, metadata?: StellarChannel): string {
  return metadata?.marketing_name || cleanChannelName(channel.name);
}

/**
 * Prefers StellarTunerLog's channel number, then the one the provider baked
 * into the name - Xtream's own `num` is a position in the provider's lineup
 * (3293), not a station number, so it is the last resort.
 */
export function channelDisplayNumber(channel: XtreamChannel, metadata?: StellarChannel): number {
  return metadata?.channel_number ?? parseChannelName(channel.name).number ?? channel.num;
}

/**
 * Whether a channel found no StellarTunerLog metadata. Only meaningful once
 * the metadata fetch has actually completed - while it is loading (or after it
 * failed) every channel is metadata-less, and flagging them all would be noise.
 */
export function isChannelUnmatched(
  metadata: StellarChannel | undefined,
  metadataStatus: 'idle' | 'loading' | 'loaded' | 'error',
): boolean {
  return metadataStatus === 'loaded' && !metadata;
}
