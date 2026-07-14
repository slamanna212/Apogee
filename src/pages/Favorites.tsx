import { useMemo } from 'react';
import { Text } from '@mantine/core';
import { useChannelStore } from '../stores/channelStore';
import { useLibraryStore } from '../stores/libraryStore';
import { usePlayerStore } from '../stores/playerStore';
import { ChannelGrid } from '../components/ChannelGrid';

interface FavoritesProps {
  onSelectChannel: (streamId: number) => void;
  onPlayChannel: (streamId: number) => void;
}

export function Favorites({ onSelectChannel, onPlayChannel }: FavoritesProps) {
  const allChannels = useChannelStore((s) => s.channels);
  const channelMetadata = useChannelStore((s) => s.channelMetadata);
  const nowPlaying = useChannelStore((s) => s.nowPlaying);
  const favorites = useLibraryStore((s) => s.favorites);
  const sortMode = useLibraryStore((s) => s.sortMode);
  const setSortMode = useLibraryStore((s) => s.setSortMode);
  const viewMode = useLibraryStore((s) => s.viewMode);
  const setViewMode = useLibraryStore((s) => s.setViewMode);
  const toggleFavorite = useLibraryStore((s) => s.toggleFavorite);
  const currentChannelId = usePlayerStore((s) => s.currentChannel?.stream_id);

  const channels = useMemo(
    () => allChannels.filter((c) => favorites.includes(c.stream_id)),
    [allChannels, favorites],
  );

  return (
    <ChannelGrid
      title="Favorites"
      channels={channels}
      channelMetadata={channelMetadata}
      nowPlaying={nowPlaying}
      favorites={favorites}
      sortMode={sortMode}
      onSortModeChange={setSortMode}
      viewMode={viewMode}
      onViewModeChange={setViewMode}
      onToggleFavorite={toggleFavorite}
      onSelect={onSelectChannel}
      onPlay={onPlayChannel}
      currentChannelId={currentChannelId}
      emptyState={<Text c="dimmed">Hover any channel and tap the star to save it here.</Text>}
    />
  );
}
