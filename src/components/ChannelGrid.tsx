import { useMemo, useRef, type ReactNode } from 'react';
import { IconLayoutGrid, IconLayoutList } from '@tabler/icons-react';
import { ChannelCard } from './ChannelCard';
import { ChannelListRow } from './ChannelListRow';
import { JumpRail } from './JumpRail';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarStation } from '../types/stellarTunerLog';
import type { SortMode, ViewMode } from '../stores/libraryStore';

interface ChannelGridProps {
  title: string;
  channels: XtreamChannel[];
  channelMetadata: Map<number, StellarChannel>;
  favorites: number[];
  onToggleFavorite: (streamId: number) => void;
  onSelect: (streamId: number) => void;
  onPlay: (streamId: number) => void;
  /** Recent/Favorites keep their own fixed order (recency / favorite order) instead of the shared sort toggle. */
  sortMode?: SortMode;
  onSortModeChange?: (mode: SortMode) => void;
  viewMode: ViewMode;
  onViewModeChange: (mode: ViewMode) => void;
  nowPlaying?: Map<number, StellarStation>;
  emptyState?: ReactNode;
}

export function ChannelGrid({
  title,
  channels,
  channelMetadata,
  favorites,
  onToggleFavorite,
  onSelect,
  onPlay,
  sortMode,
  onSortModeChange,
  viewMode,
  onViewModeChange,
  nowPlaying,
  emptyState,
}: ChannelGridProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const favoriteSet = useMemo(() => new Set(favorites), [favorites]);
  const sortable = sortMode != null && onSortModeChange != null;

  const sorted = useMemo(() => {
    if (!sortable) return channels;
    const list = [...channels];
    if (sortMode === 'az') {
      list.sort((a, b) =>
        (channelMetadata.get(a.stream_id)?.marketing_name || a.name).localeCompare(
          channelMetadata.get(b.stream_id)?.marketing_name || b.name,
        ),
      );
    } else {
      list.sort(
        (a, b) =>
          (channelMetadata.get(a.stream_id)?.channel_number ?? a.num) -
          (channelMetadata.get(b.stream_id)?.channel_number ?? b.num),
      );
    }
    return list;
  }, [channels, channelMetadata, sortMode, sortable]);

  const groups = useMemo(() => {
    if (!sortable) return [];
    const seen = new Set<string>();
    const result: { label: string; index: number }[] = [];
    sorted.forEach((channel, index) => {
      const label =
        sortMode === 'az'
          ? (channelMetadata.get(channel.stream_id)?.marketing_name || channel.name).charAt(0).toUpperCase()
          : `${Math.floor((channelMetadata.get(channel.stream_id)?.channel_number ?? channel.num) / 10) * 10}`;
      if (!seen.has(label)) {
        seen.add(label);
        result.push({ label, index });
      }
    });
    return result;
  }, [sorted, sortMode, channelMetadata, sortable]);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 20, paddingRight: sortable ? 70 : 0 }}>
        <div style={{ font: '700 24px "Space Grotesk", sans-serif' }}>
          {title} <span style={{ color: 'var(--app-dim2)', font: '400 14px "Sora", sans-serif' }}>{channels.length}</span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <div style={{ display: 'flex', background: 'var(--app-panel)', border: '1px solid var(--app-border)', borderRadius: 999, padding: 3 }}>
            {(['list', 'grid'] as const).map((mode) => (
              <div
                key={mode}
                onClick={() => onViewModeChange(mode)}
                role="button"
                aria-label={mode === 'grid' ? 'Grid view' : 'List view'}
                style={{
                  width: 30,
                  height: 26,
                  borderRadius: 999,
                  cursor: 'pointer',
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  background: viewMode === mode ? 'var(--app-accent)' : 'transparent',
                  color: viewMode === mode ? '#07060d' : 'var(--app-dim)',
                }}
              >
                {mode === 'grid' ? <IconLayoutGrid size={15} /> : <IconLayoutList size={15} />}
              </div>
            ))}
          </div>
          {sortable && (
            <div style={{ display: 'flex', background: 'var(--app-panel)', border: '1px solid var(--app-border)', borderRadius: 999, padding: 3 }}>
              {(['channel_number', 'az'] as const).map((mode) => (
                <div
                  key={mode}
                  onClick={() => onSortModeChange!(mode)}
                  role="button"
                  style={{
                    padding: '6px 14px',
                    borderRadius: 999,
                    cursor: 'pointer',
                    background: sortMode === mode ? 'var(--app-accent)' : 'transparent',
                    color: sortMode === mode ? '#07060d' : 'var(--app-dim)',
                    font: '600 12px "Sora", sans-serif',
                  }}
                >
                  {mode === 'az' ? 'A–Z' : 'Channel #'}
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
      {sorted.length === 0 && emptyState ? (
        emptyState
      ) : (
        <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
          <div ref={containerRef} style={{ flex: 1, overflowY: 'auto', paddingRight: 20 }}>
            {viewMode === 'grid' ? (
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))', gap: 22 }}>
                {sorted.map((channel) => (
                  <ChannelCard
                    key={channel.stream_id}
                    channel={channel}
                    metadata={channelMetadata.get(channel.stream_id)}
                    isFavorite={favoriteSet.has(channel.stream_id)}
                    onToggleFavorite={() => onToggleFavorite(channel.stream_id)}
                    onClick={() => onPlay(channel.stream_id)}
                    onInfo={() => onSelect(channel.stream_id)}
                  />
                ))}
              </div>
            ) : (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
                {sorted.map((channel) => (
                  <ChannelListRow
                    key={channel.stream_id}
                    channel={channel}
                    metadata={channelMetadata.get(channel.stream_id)}
                    isFavorite={favoriteSet.has(channel.stream_id)}
                    onToggleFavorite={() => onToggleFavorite(channel.stream_id)}
                    onClick={() => onPlay(channel.stream_id)}
                    onInfo={() => onSelect(channel.stream_id)}
                    nowPlaying={nowPlaying?.get(channel.stream_id)}
                  />
                ))}
              </div>
            )}
          </div>
          {sortable && <JumpRail groups={groups} totalCount={sorted.length} containerRef={containerRef} />}
        </div>
      )}
    </div>
  );
}
