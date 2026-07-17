import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarStation } from '../types/stellarTunerLog';

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
  const channelName = searchableText(metadata?.marketing_name || channel.name);
  if (channelName.includes(normalizedQuery)) return true;

  return (
    searchableText(station?.title).includes(normalizedQuery) ||
    searchableText(station?.artist).includes(normalizedQuery)
  );
}
