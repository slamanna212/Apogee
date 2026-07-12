import { useState } from 'react';
import { Text } from '@mantine/core';
import { IconPlayerPlayFilled, IconStar, IconStarFilled } from '@tabler/icons-react';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarStation } from '../types/stellarTunerLog';

interface ChannelListRowProps {
  channel: XtreamChannel;
  metadata?: StellarChannel;
  isFavorite: boolean;
  onToggleFavorite: () => void;
  onClick: () => void;
  onPlay: () => void;
  nowPlaying?: StellarStation;
}

export function ChannelListRow({
  channel,
  metadata,
  isFavorite,
  onToggleFavorite,
  onClick,
  onPlay,
  nowPlaying,
}: ChannelListRowProps) {
  const [hovered, setHovered] = useState(false);
  const name = metadata?.marketing_name || channel.name;
  const number = metadata?.channel_number ?? channel.num;
  const logoUrl = metadata?.logos?.color_dark_square?.url || channel.stream_icon;
  const subtitle = [nowPlaying?.artist, nowPlaying?.title].filter(Boolean).join(' — ');

  return (
    <div
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onClick={onClick}
      role="button"
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 16,
        background: 'var(--app-panel)',
        border: '1px solid var(--app-border)',
        borderRadius: 16,
        padding: 14,
        cursor: 'pointer',
      }}
    >
      <div style={{ width: 64, height: 64, flex: 'none', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        {logoUrl ? (
          <img src={logoUrl} alt="" style={{ width: '100%', height: '100%', objectFit: 'contain' }} />
        ) : (
          <div style={{ width: '70%', height: '70%', borderRadius: 8, background: 'var(--app-panel2)' }} />
        )}
      </div>
      <div style={{ width: 34, flex: 'none', textAlign: 'right', font: '600 13px "Sora", sans-serif', color: 'var(--app-dim)' }}>
        {number}
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <Text size="md" fw={600} truncate>
          {name}
        </Text>
        <Text size="sm" c="dimmed" truncate>
          {subtitle}
        </Text>
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, flex: 'none' }}>
        <div
          onClick={(e) => {
            e.stopPropagation();
            onToggleFavorite();
          }}
          role="button"
          aria-label={isFavorite ? 'Remove favorite' : 'Add favorite'}
          style={{
            width: 32,
            height: 32,
            borderRadius: '50%',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: isFavorite ? 'var(--app-accent)' : 'var(--app-dim)',
            opacity: isFavorite || hovered ? 1 : 0,
            transition: 'opacity 150ms',
          }}
        >
          {isFavorite ? <IconStarFilled size={16} /> : <IconStar size={16} />}
        </div>
        <div
          onClick={(e) => {
            e.stopPropagation();
            onPlay();
          }}
          role="button"
          aria-label={`Play ${name}`}
          style={{
            width: 32,
            height: 32,
            borderRadius: '50%',
            background: 'var(--app-accent)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: '#07060d',
            opacity: hovered ? 1 : 0,
            transition: 'opacity 150ms',
          }}
        >
          <IconPlayerPlayFilled size={16} />
        </div>
      </div>
    </div>
  );
}
