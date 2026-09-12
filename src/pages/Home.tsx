import { memo, useMemo } from 'react';
import { Text } from '@mantine/core';
import { useChannelStore } from '../stores/channelStore';
import { useSettingsStore } from '../stores/settingsStore';
import { useLibraryStore } from '../stores/libraryStore';
import { usePlayerStore } from '../stores/playerStore';
import { ChannelCard, CHANNEL_CARD_MIN_WIDTH, CHANNEL_CARD_GAP } from '../components/ChannelCard';
import { HOME_PAGE_SIZES, buildRecommendationRows, getAllGenres, rankPersonalizedGenres, shuffleGenres } from '../lib/recommendations';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarStation } from '../types/stellarTunerLog';

interface RowProps {
  title: string;
  subtitle?: string;
  channels: XtreamChannel[];
  channelMetadata: Map<number, StellarChannel>;
  nowPlaying: Map<number, StellarStation>;
  favoriteSet: Set<number>;
  currentChannelId?: number;
  onToggleFavorite: (streamId: number) => void;
  onSelect: (streamId: number) => void;
  onPlay: (streamId: number) => void;
}

const Row = memo(function Row({ title, subtitle, channels, channelMetadata, nowPlaying, favoriteSet, currentChannelId, onToggleFavorite, onSelect, onPlay }: RowProps) {
  if (channels.length === 0) return null;
  return (
    <div style={{ marginBottom: 30 }}>
      <div style={{ font: '600 13px "Sora", sans-serif', color: 'var(--app-dim)', marginBottom: 12 }}>
        {title}
        {subtitle && <span style={{ color: 'var(--app-dim2)', fontWeight: 400 }}> — {subtitle}</span>}
      </div>
      <div style={{ display: 'flex', gap: CHANNEL_CARD_GAP, overflowX: 'auto', paddingBottom: 22 }}>
        {channels.map((channel) => (
          <div key={channel.stream_id} style={{ width: CHANNEL_CARD_MIN_WIDTH, flex: 'none' }}>
            <ChannelCard
              channel={channel}
              metadata={channelMetadata.get(channel.stream_id)}
              nowPlaying={nowPlaying.get(channel.stream_id)}
              isFavorite={favoriteSet.has(channel.stream_id)}
              isPlaying={channel.stream_id === currentChannelId}
              onToggleFavorite={onToggleFavorite}
              onClick={onPlay}
              onInfo={onSelect}
            />
          </div>
        ))}
      </div>
    </div>
  );
});

interface HomeProps {
  onSelectChannel: (streamId: number) => void;
  onPlayChannel: (streamId: number) => void;
}

export function Home({ onSelectChannel, onPlayChannel }: HomeProps) {
  const channels = useChannelStore((s) => s.channels);
  const channelMetadata = useChannelStore((s) => s.channelMetadata);
  const nowPlaying = useChannelStore((s) => s.nowPlaying);
  const favorites = useLibraryStore((s) => s.favorites);
  const recentlyPlayed = useLibraryStore((s) => s.recentlyPlayed);
  const toggleFavorite = useLibraryStore((s) => s.toggleFavorite);
  const currentChannelId = usePlayerStore((s) => s.currentChannel?.stream_id);

  const homePageSize = useSettingsStore((s) => s.settings.homePageSize);
  const { maxRows, channelsPerRow, recentCount } = HOME_PAGE_SIZES[homePageSize];

  const favoriteSet = useMemo(() => new Set(favorites), [favorites]);

  const recentChannels = useMemo(
    () =>
      recentlyPlayed
        .map((id) => channels.find((c) => c.stream_id === id))
        .filter((c): c is XtreamChannel => Boolean(c)),
    [recentlyPlayed, channels],
  );

  // Stable array identity so the memoized Row isn't re-rendered every tick.
  const topRecent = useMemo(() => recentChannels.slice(0, recentCount), [recentChannels, recentCount]);

  const personalizedGenres = useMemo(
    () => rankPersonalizedGenres(channelMetadata, recentlyPlayed, favorites),
    [channelMetadata, recentlyPlayed, favorites],
  );

  const shuffledFillerGenres = useMemo(
    () => shuffleGenres(getAllGenres(channelMetadata)),
    [channelMetadata],
  );

  const recommendationRows = useMemo(
    () => buildRecommendationRows(personalizedGenres, shuffledFillerGenres, channels, channelMetadata, recentlyPlayed, maxRows, channelsPerRow),
    [personalizedGenres, shuffledFillerGenres, channels, channelMetadata, recentlyPlayed, maxRows, channelsPerRow],
  );

  return (
    <div>
      <div style={{ font: '700 26px "Space Grotesk", sans-serif', marginBottom: 2 }}>Welcome back</div>
      <Text size="xs" c="dimmed" mb={20}>
        Picked up from where you left off
      </Text>
      <Row
        title="Recently played"
        channels={topRecent}
        channelMetadata={channelMetadata}
        nowPlaying={nowPlaying}
        favoriteSet={favoriteSet}
        currentChannelId={currentChannelId}
        onToggleFavorite={toggleFavorite}
        onSelect={onSelectChannel}
        onPlay={onPlayChannel}
      />
      {recommendationRows.map((row) => (
        <Row
          key={row.genre}
          title={row.personalized ? `More ${row.genre}` : `Explore ${row.genre}`}
          subtitle={row.personalized ? `because you've been playing ${row.genre}` : 'something new to try'}
          channels={row.channels}
          channelMetadata={channelMetadata}
          nowPlaying={nowPlaying}
          favoriteSet={favoriteSet}
          currentChannelId={currentChannelId}
          onToggleFavorite={toggleFavorite}
          onSelect={onSelectChannel}
          onPlay={onPlayChannel}
        />
      ))}
      {recentChannels.length === 0 && (
        <Text c="dimmed">Nothing played yet — pick a channel from Channels to get started.</Text>
      )}
    </div>
  );
}
