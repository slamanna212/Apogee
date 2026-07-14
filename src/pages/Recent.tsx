import { useMemo } from 'react';
import { Text } from '@mantine/core';
import { useChannelStore } from '../stores/channelStore';
import { useLibraryStore } from '../stores/libraryStore';
import { usePlayerStore } from '../stores/playerStore';
import { ChannelGrid } from '../components/ChannelGrid';
import type { XtreamChannel } from '../types/xtream';

interface RecentProps {
  onSelectChannel: (streamId: number) => void;
  onPlayChannel: (streamId: number) => void;
}

export function Recent({ onSelectChannel, onPlayChannel }: RecentProps) {
  const allChannels = useChannelStore((s) => s.channels);
  const channelMetadata = useChannelStore((s) => s.channelMetadata);
  const nowPlaying = useChannelStore((s) => s.nowPlaying);
  const favorites = useLibraryStore((s) => s.favorites);
  const recentlyPlayed = useLibraryStore((s) => s.recentlyPlayed);
  const viewMode = useLibraryStore((s) => s.viewMode);
  const setViewMode = useLibraryStore((s) => s.setViewMode);
  const toggleFavorite = useLibraryStore((s) => s.toggleFavorite);
  const currentChannelId = usePlayerStore((s) => s.currentChannel?.stream_id);

  const channels = useMemo(
    () =>
      recentlyPlayed
        .map((id) => allChannels.find((c) => c.stream_id === id))
        .filter((c): c is XtreamChannel => Boolean(c)),
    [recentlyPlayed, allChannels],
  );

  return (
    <ChannelGrid
      title="Recent"
      channels={channels}
      channelMetadata={channelMetadata}
      nowPlaying={nowPlaying}
      favorites={favorites}
      viewMode={viewMode}
      onViewModeChange={setViewMode}
      onToggleFavorite={toggleFavorite}
      onSelect={onSelectChannel}
      onPlay={onPlayChannel}
      currentChannelId={currentChannelId}
      emptyState={<Text c="dimmed">Nothing played yet.</Text>}
    />
  );
}
