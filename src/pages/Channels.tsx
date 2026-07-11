import { useChannelStore } from '../stores/channelStore';
import { useLibraryStore } from '../stores/libraryStore';
import { ChannelGrid } from '../components/ChannelGrid';

interface ChannelsProps {
  onSelectChannel: (streamId: number) => void;
  onPlayChannel: (streamId: number) => void;
}

export function Channels({ onSelectChannel, onPlayChannel }: ChannelsProps) {
  const channels = useChannelStore((s) => s.channels);
  const channelMetadata = useChannelStore((s) => s.channelMetadata);
  const favorites = useLibraryStore((s) => s.favorites);
  const sortMode = useLibraryStore((s) => s.sortMode);
  const setSortMode = useLibraryStore((s) => s.setSortMode);
  const toggleFavorite = useLibraryStore((s) => s.toggleFavorite);

  return (
    <ChannelGrid
      title="All channels"
      channels={channels}
      channelMetadata={channelMetadata}
      favorites={favorites}
      sortMode={sortMode}
      onSortModeChange={setSortMode}
      onToggleFavorite={toggleFavorite}
      onSelect={onSelectChannel}
      onPlay={onPlayChannel}
    />
  );
}
