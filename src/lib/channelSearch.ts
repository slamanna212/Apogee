import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarStation } from '../types/stellarTunerLog';
import { channelDisplayName } from './channelDisplay';

function searchableText(value: unknown): string {
  return typeof value === 'string' ? value.toLowerCase() : '';
}

/**
 * Matches data that came from remote JSON without assuming its optional track
 * fields actually conform to the compile-time interfaces. A station can be
 * between songs (or return partial metadata), so null artist/title values must
 * behave like empty strings rather than taking down the React render.
 */
export function channelMatchesSearch(
  channel: XtreamChannel,
  metadata: StellarChannel | undefined,
  station: StellarStation | undefined,
  normalizedQuery: string,
): boolean {
  // Both the raw provider name and the name actually shown, so a query still
  // finds a channel by whatever the provider called it ("43 Rock The Bells")
  // as well as by its displayed name.
  if (searchableText(channel.name).includes(normalizedQuery)) return true;
  if (searchableText(channelDisplayName(channel, metadata)).includes(normalizedQuery)) return true;

  return (
    searchableText(station?.title).includes(normalizedQuery) ||
    searchableText(station?.artist).includes(normalizedQuery)
  );
}
