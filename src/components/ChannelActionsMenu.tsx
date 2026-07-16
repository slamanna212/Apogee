import { useState, type CSSProperties, type MouseEvent } from 'react';
import { Menu, type FloatingPosition } from '@mantine/core';
import { IconDotsVertical, IconMoon, IconMoonFilled, IconStar, IconStarFilled } from '@tabler/icons-react';
import type { StellarStation } from '../types/stellarTunerLog';
import { useTrackFollowState } from '../hooks/useTrackFollowState';
import { useSleepTimerStore } from '../stores/sleepTimerStore';
import { useSettingsStore } from '../stores/settingsStore';

const SLEEP_TIMER_DURATIONS_MINUTES = [15, 30, 45, 60];

function formatRemaining(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${m}:${s.toString().padStart(2, '0')}`;
}

function SleepTimerMenu({
  itemStyle,
  horizontal,
  onSelect,
}: {
  itemStyle?: CSSProperties;
  horizontal: boolean;
  onSelect: () => void;
}) {
  const remainingSeconds = useSleepTimerStore((s) => s.remainingSeconds);
  const start = useSleepTimerStore((s) => s.start);
  const cancel = useSleepTimerStore((s) => s.cancel);
  const lastSleepTimerMinutes = useSettingsStore((s) => s.settings.lastSleepTimerMinutes);
  const active = remainingSeconds != null;

  function stop(e: MouseEvent, action: () => void) {
    e.stopPropagation();
    action();
    onSelect();
  }

  return (
    <Menu trigger="hover" openDelay={100} closeDelay={200} position={horizontal ? 'left-start' : 'right-start'} withinPortal offset={4}>
      <Menu.Target>
        <Menu.Item
          style={itemStyle}
          leftSection={active ? <IconMoonFilled size={14} style={{ color: 'var(--app-accent)' }} /> : <IconMoon size={14} />}
        >
          {active ? `Sleep timer · ${formatRemaining(remainingSeconds)}` : 'Sleep timer'}
        </Menu.Item>
      </Menu.Target>
      <Menu.Dropdown>
        {active ? (
          <Menu.Item onClick={(e) => stop(e, cancel)}>Cancel timer</Menu.Item>
        ) : (
          SLEEP_TIMER_DURATIONS_MINUTES.map((minutes) => (
            <Menu.Item
              key={minutes}
              onClick={(e) => stop(e, () => start(minutes))}
              style={{ color: minutes === lastSleepTimerMinutes ? 'var(--app-accent)' : undefined }}
            >
              {minutes} min
            </Menu.Item>
          ))
        )}
      </Menu.Dropdown>
    </Menu>
  );
}

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
  const [opened, setOpened] = useState(false);
  const horizontal = layout === 'horizontal';
  const itemStyle = horizontal ? { width: 'auto', flex: 'none', whiteSpace: 'nowrap' as const } : undefined;

  function stop(e: MouseEvent, action: () => void) {
    e.stopPropagation();
    action();
  }

  return (
    <Menu opened={opened} onChange={setOpened} position={horizontal ? 'left' : position} withinPortal offset={8}>
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
        <SleepTimerMenu itemStyle={itemStyle} horizontal={horizontal} onSelect={() => setOpened(false)} />
        {horizontal ? (
          <div style={{ width: 1, height: 24, margin: '0 2px', background: 'var(--app-border)', flex: 'none' }} />
        ) : (
          <Menu.Divider />
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
