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
}

export function ChannelActionsMenu({
  nowPlaying,
  isFavorite,
  onToggleFavorite,
  triggerStyle,
  onMouseEnter,
  onMouseLeave,
  position = 'bottom-end',
}: ChannelActionsMenuProps) {
  const { trackEntry, artistEntry, followTrack, followArtist, unfollowTrack, unfollowArtist } = useTrackFollowState(nowPlaying);

  function stop(e: MouseEvent, action: () => void) {
    e.stopPropagation();
    action();
  }

  return (
    <Menu position={position} withinPortal>
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
      <Menu.Dropdown>
        {nowPlaying && (
          <>
            {trackEntry ? (
              <Menu.Item onClick={(e) => stop(e, unfollowTrack)}>Unfollow track</Menu.Item>
            ) : (
              <Menu.Item onClick={(e) => stop(e, followTrack)}>Follow track</Menu.Item>
            )}
            {artistEntry ? (
              <Menu.Item onClick={(e) => stop(e, unfollowArtist)}>Unfollow artist</Menu.Item>
            ) : (
              <Menu.Item onClick={(e) => stop(e, followArtist)}>Follow artist</Menu.Item>
            )}
            <Menu.Divider />
          </>
        )}
        <Menu.Item
          leftSection={isFavorite ? <IconStarFilled size={14} /> : <IconStar size={14} />}
          onClick={(e) => stop(e, onToggleFavorite)}
        >
          {isFavorite ? 'Remove favorite' : 'Add favorite'}
        </Menu.Item>
      </Menu.Dropdown>
    </Menu>
  );
}
