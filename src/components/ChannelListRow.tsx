import { Text } from '@mantine/core';
import { IconPlayerPlayFilled, IconStar, IconStarFilled } from '@tabler/icons-react';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarStation } from '../types/stellarTunerLog';
import { CutTypeBadge } from './CutTypeBadge';

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
  const name = metadata?.marketing_name || channel.name;
  const number = metadata?.channel_number ?? channel.num;
  const logoUrl = metadata?.logos?.color_dark_square?.url || channel.stream_icon;
  const artworkUrl = nowPlaying?.artwork_url;
  const trackTitle = nowPlaying?.title;
  const trackArtist = nowPlaying?.artist;

  return (
    <div
      onClick={onClick}
      role="button"
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 16,
        background: 'var(--app-panel)',
        border: '1px solid var(--app-border)',
        borderRadius: 16,
        padding: '6px 14px',
        cursor: 'pointer',
      }}
    >
      <div style={{ width: 92, flex: 'none', display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 6 }}>
        <div style={{ width: 64, height: 64, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          {logoUrl ? (
            <img src={logoUrl} alt="" style={{ width: '100%', height: '100%', objectFit: 'contain' }} />
          ) : (
            <div style={{ width: '70%', height: '70%', borderRadius: 8, background: 'var(--app-panel2)' }} />
          )}
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
          style={{ width: 56, height: 56, borderRadius: 14, objectFit: 'cover', flex: 'none', background: 'var(--app-panel2)' }}
        />
      ) : (
        <div style={{ width: 56, height: 56, borderRadius: 14, background: 'var(--app-panel2)', flex: 'none' }} />
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
            width: 42,
            height: 42,
            borderRadius: '50%',
            background: 'var(--app-accent)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: '#07060d',
          }}
        >
          <IconPlayerPlayFilled size={20} />
        </div>
      </div>
    </div>
  );
}
