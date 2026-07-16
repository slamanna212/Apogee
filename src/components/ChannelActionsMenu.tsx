import type { CSSProperties, MouseEvent } from 'react';
import { Menu, type FloatingPosition } from '@mantine/core';
import { IconDotsVertical, IconStar, IconStarFilled } from '@tabler/icons-react';
import type { StellarStation } from '../types/stellarTunerLog';
import { useTrackFollowState } from '../hooks/useTrackFollowState';

interface ChannelActionsMenuProps {
  nowPlaying?: StellarStation;
  isFavorite: boolean;
  onToggleFavorite: () => void;
  triggerStyle: CSSProperties;
  onMouseEnter?: () => void;
  onMouseLeave?: () => void;
  position?: FloatingPosition;
  layout?: 'vertical' | 'horizontal';
}

export function ChannelActionsMenu({
  nowPlaying,
  isFavorite,
  onToggleFavorite,
  triggerStyle,
  onMouseEnter,
  onMouseLeave,
  position = 'bottom-end',
  layout = 'vertical',
}: ChannelActionsMenuProps) {
  const { trackEntry, artistEntry, followTrack, followArtist, unfollowTrack, unfollowArtist } = useTrackFollowState(nowPlaying);
  const horizontal = layout === 'horizontal';
  const itemStyle = horizontal ? { width: 'auto', flex: 'none', whiteSpace: 'nowrap' as const } : undefined;

  function stop(e: MouseEvent, action: () => void) {
    e.stopPropagation();
    action();
  }

  return (
    <Menu position={horizontal ? 'left' : position} withinPortal offset={8}>
      <Menu.Target>
        <div
          onClick={(e) => e.stopPropagation()}
          onMouseEnter={onMouseEnter}
          onMouseLeave={onMouseLeave}
          role="button"
          aria-label="More options"
          style={triggerStyle}
        >
          <IconDotsVertical size={16} />
        </div>
      </Menu.Target>
      <Menu.Dropdown
        style={horizontal ? { display: 'flex', alignItems: 'center', gap: 4, padding: 6 } : undefined}
      >
        {nowPlaying && (
          <>
            {trackEntry ? (
              <Menu.Item style={itemStyle} onClick={(e) => stop(e, unfollowTrack)}>Unfollow track</Menu.Item>
            ) : (
              <Menu.Item style={itemStyle} onClick={(e) => stop(e, followTrack)}>Follow track</Menu.Item>
            )}
            {artistEntry ? (
              <Menu.Item style={itemStyle} onClick={(e) => stop(e, unfollowArtist)}>Unfollow artist</Menu.Item>
            ) : (
              <Menu.Item style={itemStyle} onClick={(e) => stop(e, followArtist)}>Follow artist</Menu.Item>
            )}
            {horizontal ? (
              <div style={{ width: 1, height: 24, margin: '0 2px', background: 'var(--app-border)', flex: 'none' }} />
            ) : (
              <Menu.Divider />
            )}
          </>
        )}
        <Menu.Item
          style={itemStyle}
          leftSection={isFavorite ? <IconStarFilled size={14} /> : <IconStar size={14} />}
          onClick={(e) => stop(e, onToggleFavorite)}
        >
          {isFavorite ? 'Remove favorite' : 'Add favorite'}
        </Menu.Item>
      </Menu.Dropdown>
    </Menu>
  );
}
