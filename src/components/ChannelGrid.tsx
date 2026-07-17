import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { Text } from '@mantine/core';
import { IconLayoutGrid, IconLayoutList, IconSearch, IconX } from '@tabler/icons-react';
import { debug as logDebug } from '@tauri-apps/plugin-log';
import { ChannelCard, CHANNEL_CARD_MIN_WIDTH, CHANNEL_CARD_GAP } from './ChannelCard';
import { ChannelListRow } from './ChannelListRow';
import { JumpRail } from './JumpRail';
import { channelMatchesSearch } from '../lib/channelSearch';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarStation } from '../types/stellarTunerLog';
import type { SortMode, ViewMode } from '../stores/libraryStore';

const SEARCH_DEBOUNCE_MS = 150;

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
  currentChannelId?: number;
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
  currentChannelId,
}: ChannelGridProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const favoriteSet = useMemo(() => new Set(favorites), [favorites]);
  const sortable = sortMode != null && onSortModeChange != null;
  const [searchTerm, setSearchTerm] = useState('');
  const [debouncedSearchTerm, setDebouncedSearchTerm] = useState('');

  useEffect(() => {
    const id = setTimeout(() => setDebouncedSearchTerm(searchTerm), SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(id);
  }, [searchTerm]);

  const filtered = useMemo(() => {
    const q = debouncedSearchTerm.trim().toLowerCase();
    if (!q) return channels;
    return channels.filter((channel) =>
      channelMatchesSearch(
        channel,
        channelMetadata.get(channel.stream_id),
        nowPlaying?.get(channel.stream_id),
        q,
      ),
    );
  }, [channels, channelMetadata, debouncedSearchTerm, nowPlaying]);

  useEffect(() => {
    if (debouncedSearchTerm) {
      logDebug(`search: "${debouncedSearchTerm}" -> ${filtered.length}/${channels.length} channels`);
    }
  }, [debouncedSearchTerm, filtered.length, channels.length]);

  const sorted = useMemo(() => {
    if (!sortable) return filtered;
    const list = [...filtered];
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
  }, [filtered, channelMetadata, sortMode, sortable]);

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

  // Virtualization: only the visible cards/rows are mounted, so a full-category
  // list (hundreds of channels, each an expensive backdrop-filter card) doesn't
  // pay layout/paint for offscreen items. The scroll container (containerRef) is
  // the virtualizer's scroll element in both modes.
  const [containerWidth, setContainerWidth] = useState(0);
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    setContainerWidth(el.clientWidth);
    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width;
      if (w != null) setContainerWidth(w);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // clientWidth already excludes the container's own left/right padding, so the
  // grid columns follow the same auto-fill math the CSS grid would use.
  const columns = Math.max(
    1,
    Math.floor((containerWidth + CHANNEL_CARD_GAP) / (CHANNEL_CARD_MIN_WIDTH + CHANNEL_CARD_GAP)),
  );
  const lanes = viewMode === 'grid' ? columns : 1;
  const rowCount = Math.ceil(sorted.length / lanes);
  // Grid rows are ~square card + label block; list rows are the fixed ChannelListRow.
  // Real heights are re-measured after mount via measureElement, so these are just
  // the initial estimate that keeps the scrollbar roughly right before measurement.
  const estimatedRowHeight =
    viewMode === 'grid'
      ? Math.round((containerWidth - CHANNEL_CARD_GAP * (columns - 1)) / columns) + 62
      : 122;

  const rowVirtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => containerRef.current,
    estimateSize: () => estimatedRowHeight,
    overscan: 4,
    gap: viewMode === 'grid' ? CHANNEL_CARD_GAP : 10,
  });

  // Re-measure when the layout basis changes (mode switch, column count, filter).
  useEffect(() => {
    rowVirtualizer.measure();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewMode, columns, sorted.length]);

  const scrollToItemIndex = useCallback(
    (itemIndex: number) => {
      rowVirtualizer.scrollToIndex(Math.floor(itemIndex / lanes), { align: 'start' });
    },
    [rowVirtualizer, lanes],
  );

  return (
    <div style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 20, paddingRight: sortable && !searchTerm ? 70 : 0 }}>
        <div style={{ font: '700 24px "Space Grotesk", sans-serif' }}>
          {title} <span style={{ color: 'var(--app-dim2)', font: '400 14px "Sora", sans-serif' }}>{sorted.length}</span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, background: 'var(--app-panel)', border: '1px solid var(--app-border)', borderRadius: 999, padding: '0 12px', height: 32, width: 160 }}>
            <IconSearch size={14} style={{ color: 'var(--app-dim)', flex: 'none' }} />
            <input
              value={searchTerm}
              onChange={(e) => setSearchTerm(e.target.value)}
              placeholder="Search channels, songs, artists"
              style={{ background: 'transparent', border: 'none', outline: 'none', color: 'var(--app-text)', font: '400 12px "Sora", sans-serif', width: '100%' }}
            />
            {searchTerm && (
              <IconX
                size={14}
                role="button"
                aria-label="Clear search"
                onClick={() => setSearchTerm('')}
                style={{ color: 'var(--app-dim)', cursor: 'pointer', flex: 'none' }}
              />
            )}
          </div>
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
                  color: viewMode === mode ? 'var(--app-bg)' : 'var(--app-dim)',
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
                    color: sortMode === mode ? 'var(--app-bg)' : 'var(--app-dim)',
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
      {channels.length === 0 && emptyState ? (
        emptyState
      ) : sorted.length === 0 && searchTerm ? (
        <Text c="dimmed">No channels match &quot;{searchTerm}&quot;.</Text>
      ) : (
        <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
          <div ref={containerRef} style={{ flex: 1, overflowY: 'auto', padding: '4px 20px 4px 4px' }}>
            <div style={{ position: 'relative', height: rowVirtualizer.getTotalSize(), width: '100%' }}>
              {rowVirtualizer.getVirtualItems().map((virtualRow) => {
                const rowItems = sorted.slice(virtualRow.index * lanes, virtualRow.index * lanes + lanes);
                return (
                  <div
                    key={virtualRow.key}
                    data-index={virtualRow.index}
                    ref={rowVirtualizer.measureElement}
                    style={{
                      position: 'absolute',
                      top: 0,
                      left: 0,
                      width: '100%',
                      transform: `translateY(${virtualRow.start}px)`,
                    }}
                  >
                    {viewMode === 'grid' ? (
                      <div style={{ display: 'grid', gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`, gap: CHANNEL_CARD_GAP }}>
                        {rowItems.map((channel) => (
                          <ChannelCard
                            key={channel.stream_id}
                            channel={channel}
                            metadata={channelMetadata.get(channel.stream_id)}
                            isFavorite={favoriteSet.has(channel.stream_id)}
                            isPlaying={channel.stream_id === currentChannelId}
                            onToggleFavorite={onToggleFavorite}
                            onClick={onPlay}
                            onInfo={onSelect}
                            nowPlaying={nowPlaying?.get(channel.stream_id)}
                          />
                        ))}
                      </div>
                    ) : (
                      rowItems.map((channel) => (
                        <ChannelListRow
                          key={channel.stream_id}
                          channel={channel}
                          metadata={channelMetadata.get(channel.stream_id)}
                          isFavorite={favoriteSet.has(channel.stream_id)}
                          isPlaying={channel.stream_id === currentChannelId}
                          onToggleFavorite={onToggleFavorite}
                          onClick={onPlay}
                          onInfo={onSelect}
                          nowPlaying={nowPlaying?.get(channel.stream_id)}
                        />
                      ))
                    )}
                  </div>
                );
              })}
            </div>
          </div>
          {sortable && !searchTerm && (
            <JumpRail groups={groups} totalCount={sorted.length} containerRef={containerRef} onJump={scrollToItemIndex} />
          )}
        </div>
      )}
    </div>
  );
}
