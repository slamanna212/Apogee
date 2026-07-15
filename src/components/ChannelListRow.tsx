import { useState } from 'react';
import { Text, useComputedColorScheme } from '@mantine/core';
import { IconInfoCircle, IconPlayerPlayFilled } from '@tabler/icons-react';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarStation } from '../types/stellarTunerLog';
import { CutTypeBadge } from './CutTypeBadge';
import { ChannelActionsMenu } from './ChannelActionsMenu';
import { pickChannelLogoUrl } from '../lib/channelLogo';

interface ChannelListRowProps {
  channel: XtreamChannel;
  metadata?: StellarChannel;
  isFavorite: boolean;
  isPlaying?: boolean;
  onToggleFavorite: () => void;
  onClick: () => void;
  onInfo: () => void;
  nowPlaying?: StellarStation;
}

export function ChannelListRow({
  channel,
  metadata,
  isFavorite,
  isPlaying,
  onToggleFavorite,
  onClick,
  onInfo,
  nowPlaying,
}: ChannelListRowProps) {
  const [hovered, setHovered] = useState(false);
  const [actionHovered, setActionHovered] = useState(false);
  const showPlayButton = hovered && !actionHovered;
  const colorScheme = useComputedColorScheme('dark');
  const name = metadata?.marketing_name || channel.name;
  const number = metadata?.channel_number ?? channel.num;
  const logoUrl = pickChannelLogoUrl(metadata?.logos, colorScheme) || channel.stream_icon;
  const artworkUrl = nowPlaying?.artwork_url;
  const trackTitle = nowPlaying?.title;
  const trackArtist = nowPlaying?.artist;

  return (
    <div
      onClick={onClick}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      role="button"
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 16,
        background: isPlaying ? 'var(--app-accent-soft)' : 'var(--app-panel)',
        border: `1px solid ${isPlaying ? 'var(--app-accent)' : 'var(--app-border)'}`,
        borderRadius: 16,
        padding: '6px 14px',
        cursor: 'pointer',
      }}
    >
      <div style={{ width: 92, flex: 'none', display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 6 }}>
        <div style={{ position: 'relative', width: 64, height: 64, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          {logoUrl ? (
            <img src={logoUrl} alt="" style={{ width: '100%', height: '100%', objectFit: 'contain' }} />
          ) : (
            <div style={{ width: '70%', height: '70%', borderRadius: 8, background: 'var(--app-panel2)' }} />
          )}
          <div
            style={{
              position: 'absolute',
              top: '50%',
              left: '50%',
              transform: showPlayButton ? 'translate(-50%, -50%) scale(1)' : 'translate(-50%, -50%) scale(0.85)',
              width: 36,
              height: 36,
              borderRadius: '50%',
              background: 'rgba(7,6,13,.55)',
              backdropFilter: 'blur(6px)',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              color: '#fff',
              opacity: showPlayButton ? 1 : 0,
              transition: 'opacity 150ms, transform 150ms',
            }}
          >
            <IconPlayerPlayFilled size={16} />
          </div>
        </div>
        <Text
          size="xs"
          fw={600}
          ta="center"
          style={{
            width: '100%',
            display: '-webkit-box',
            WebkitLineClamp: 2,
            WebkitBoxOrient: 'vertical',
            overflow: 'hidden',
            lineHeight: 1.25,
          }}
        >
          {name}
        </Text>
      </div>
      <div style={{ width: 48, flex: 'none', textAlign: 'center', font: '800 26px "Space Grotesk", sans-serif', color: 'var(--app-dim)' }}>
        {number}
      </div>
      {artworkUrl ? (
        <img
          src={artworkUrl}
          alt=""
          style={{ width: 68, height: 68, borderRadius: 16, objectFit: 'cover', flex: 'none', background: 'var(--app-panel2)' }}
        />
      ) : (
        <div style={{ width: 68, height: 68, borderRadius: 16, background: 'var(--app-panel2)', flex: 'none' }} />
      )}
      <div style={{ flex: 1, minWidth: 0, display: 'flex', alignItems: 'center', gap: 10 }}>
        <div style={{ minWidth: 0, maxWidth: 320, flex: 'none' }}>
          <div
            style={{
              font: '700 16px "Space Grotesk", sans-serif',
              color: 'var(--app-text)',
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
            }}
          >
            {trackTitle || '—'}
          </div>
          {trackArtist && (
            <div
              style={{
                font: '400 13px "Sora", sans-serif',
                color: 'var(--app-dim)',
                whiteSpace: 'nowrap',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
              }}
            >
              {trackArtist}
            </div>
          )}
        </div>
        <CutTypeBadge cutType={nowPlaying?.cut_type} />
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 6, flex: 'none' }}>
        <ChannelActionsMenu
          nowPlaying={nowPlaying}
          isFavorite={isFavorite}
          onToggleFavorite={onToggleFavorite}
          onMouseEnter={() => setActionHovered(true)}
          onMouseLeave={() => setActionHovered(false)}
          triggerStyle={{
            width: 32,
            height: 32,
            borderRadius: '50%',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: isFavorite ? 'var(--app-accent)' : 'var(--app-dim)',
          }}
        />
        <div
          onClick={(e) => {
            e.stopPropagation();
            onInfo();
          }}
          onMouseEnter={() => setActionHovered(true)}
          onMouseLeave={() => setActionHovered(false)}
          role="button"
          aria-label={`Info for ${name}`}
          style={{
            width: 32,
            height: 32,
            borderRadius: '50%',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: 'var(--app-dim)',
          }}
        >
          <IconInfoCircle size={20} />
        </div>
      </div>
    </div>
  );
}
