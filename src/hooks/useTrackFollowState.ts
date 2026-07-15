import { useAlertsStore } from '../stores/alertsStore';
import { normalizeText, normalizeTitle } from '../lib/songMatcher';
import type { StellarStation } from '../types/stellarTunerLog';

export function useTrackFollowState(nowPlaying?: StellarStation) {
  const entries = useAlertsStore((s) => s.entries);
  const followTrack = useAlertsStore((s) => s.followTrack);
  const followArtist = useAlertsStore((s) => s.followArtist);
  const unfollow = useAlertsStore((s) => s.unfollow);

  const trackEntry = nowPlaying
    ? entries.find(
        (e) =>
          e.type === 'track' &&
          normalizeText(e.artist) === normalizeText(nowPlaying.artist) &&
          normalizeTitle(e.title ?? '') === normalizeTitle(nowPlaying.title),
      )
    : undefined;
  const artistEntry = nowPlaying
    ? entries.find((e) => e.type === 'artist' && normalizeText(e.artist) === normalizeText(nowPlaying.artist))
    : undefined;

  return {
    trackEntry,
    artistEntry,
    followTrack: () => nowPlaying && followTrack(nowPlaying.artist, nowPlaying.title),
    followArtist: () => nowPlaying && followArtist(nowPlaying.artist),
    unfollowTrack: () => trackEntry && unfollow(trackEntry.id),
    unfollowArtist: () => artistEntry && unfollow(artistEntry.id),
  };
}
