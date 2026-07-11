import { useMemo } from 'react';
import { Text } from '@mantine/core';
import { useChannelStore } from '../stores/channelStore';
import { useLibraryStore } from '../stores/libraryStore';
import { ChannelCard } from '../components/ChannelCard';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel } from '../types/stellarTunerLog';

interface RowProps {
  title: string;
  subtitle?: string;
  channels: XtreamChannel[];
  channelMetadata: Map<number, StellarChannel>;
  favorites: number[];
  onToggleFavorite: (streamId: number) => void;
  onSelect: (streamId: number) => void;
  onPlay: (streamId: number) => void;
}

function Row({ title, subtitle, channels, channelMetadata, favorites, onToggleFavorite, onSelect, onPlay }: RowProps) {
  if (channels.length === 0) return null;
  return (
    <div style={{ marginBottom: 30 }}>
      <div style={{ font: '600 13px "Sora", sans-serif', color: 'var(--app-dim)', marginBottom: 12 }}>
        {title}
        {subtitle && <span style={{ color: 'var(--app-dim2)', fontWeight: 400 }}> — {subtitle}</span>}
      </div>
      <div style={{ display: 'flex', gap: 14, overflowX: 'auto', paddingBottom: 22 }}>
        {channels.map((channel) => (
          <div key={channel.stream_id} style={{ width: 140, flex: 'none' }}>
            <ChannelCard
              channel={channel}
              metadata={channelMetadata.get(channel.stream_id)}
              isFavorite={favorites.includes(channel.stream_id)}
              onToggleFavorite={() => onToggleFavorite(channel.stream_id)}
              onClick={() => onSelect(channel.stream_id)}
              onPlay={() => onPlay(channel.stream_id)}
            />
          </div>
        ))}
      </div>
    </div>
  );
}

interface HomeProps {
  onSelectChannel: (streamId: number) => void;
  onPlayChannel: (streamId: number) => void;
}

export function Home({ onSelectChannel, onPlayChannel }: HomeProps) {
  const channels = useChannelStore((s) => s.channels);
  const channelMetadata = useChannelStore((s) => s.channelMetadata);
  const favorites = useLibraryStore((s) => s.favorites);
  const recentlyPlayed = useLibraryStore((s) => s.recentlyPlayed);
  const toggleFavorite = useLibraryStore((s) => s.toggleFavorite);

  const recentChannels = useMemo(
    () =>
      recentlyPlayed
        .map((id) => channels.find((c) => c.stream_id === id))
        .filter((c): c is XtreamChannel => Boolean(c)),
    [recentlyPlayed, channels],
  );

  const genreRows = useMemo(() => {
    const seenGenres = new Set<string>();
    const rows: { genre: string; channels: XtreamChannel[] }[] = [];
    const recentIds = new Set(recentlyPlayed);

    for (const channel of recentChannels) {
      const meta = channelMetadata.get(channel.stream_id);
      const primary = meta?.categories.find((c) => c.is_primary);
      if (!primary || seenGenres.has(primary.name)) continue;
      seenGenres.add(primary.name);

      const related = channels.filter((c) => {
        if (recentIds.has(c.stream_id)) return false;
        return channelMetadata.get(c.stream_id)?.categories.some((cat) => cat.name === primary.name);
      });
      if (related.length > 0) rows.push({ genre: primary.name, channels: related.slice(0, 8) });
      if (rows.length >= 2) break;
    }
    return rows;
  }, [recentChannels, channelMetadata, channels, recentlyPlayed]);

  return (
    <div>
      <div style={{ font: '700 26px "Space Grotesk", sans-serif', marginBottom: 2 }}>Welcome back</div>
      <Text size="xs" c="dimmed" mb={20}>
        Picked up from where you left off
      </Text>
      <Row
        title="Recently played"
        channels={recentChannels.slice(0, 10)}
        channelMetadata={channelMetadata}
        favorites={favorites}
        onToggleFavorite={toggleFavorite}
        onSelect={onSelectChannel}
        onPlay={onPlayChannel}
      />
      {genreRows.map((row) => (
        <Row
          key={row.genre}
          title={`More ${row.genre}`}
          subtitle={`because you've been playing ${row.genre}`}
          channels={row.channels}
          channelMetadata={channelMetadata}
          favorites={favorites}
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
