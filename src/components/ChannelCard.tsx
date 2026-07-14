import { useState } from 'react';
import { IconInfoSmall, IconPlayerPlayFilled, IconStar, IconStarFilled } from '@tabler/icons-react';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel } from '../types/stellarTunerLog';

export const CHANNEL_CARD_MIN_WIDTH = 180;
export const CHANNEL_CARD_GAP = 22;

export const GRADIENTS: [string, string][] = [
  ['#ff7a5c', '#8b6bff'],
  ['#45e0d8', '#5c7cfa'],
  ['#f76707', '#f06595'],
  ['#12b886', '#339af0'],
  ['#5c7cfa', '#8b6bff'],
  ['#9775fa', '#f06595'],
  ['#fcc419', '#f76707'],
  ['#339af0', '#12b886'],
];

export function hashGradient(seed: string): string {
  let hash = 0;
  for (let i = 0; i < seed.length; i++) hash = (hash * 31 + seed.charCodeAt(i)) | 0;
  const [a, b] = GRADIENTS[Math.abs(hash) % GRADIENTS.length];
  return `linear-gradient(150deg, ${a}, ${b})`;
}

interface ChannelCardProps {
  channel: XtreamChannel;
  metadata?: StellarChannel;
  isFavorite: boolean;
  isPlaying?: boolean;
  onToggleFavorite: () => void;
  onClick: () => void;
  onInfo: () => void;
}

export function ChannelCard({ channel, metadata, isFavorite, isPlaying, onToggleFavorite, onClick, onInfo }: ChannelCardProps) {
  const [hovered, setHovered] = useState(false);
  const [actionHovered, setActionHovered] = useState(false);
  const showPlayButton = hovered && !actionHovered;
  const name = metadata?.marketing_name || channel.name;
  const number = metadata?.channel_number ?? channel.num;
  const logoUrl = metadata?.logos?.color_dark_square?.url || channel.stream_icon;
  const background = metadata?.dark_bg_color || hashGradient(channel.name);

  return (
    <div
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onClick={onClick}
      role="button"
      style={{ cursor: 'pointer' }}
    >
    <div
      style={{
        position: 'relative',
        aspectRatio: '1',
        borderRadius: 16,
        overflow: 'hidden',
        boxShadow: [
          isPlaying ? '0 0 0 2px var(--app-accent)' : null,
          hovered ? '0 8px 24px var(--app-accent-soft)' : null,
        ]
          .filter(Boolean)
          .join(', ') || 'none',
        transition: 'box-shadow 150ms',
      }}
    >
    <div
      style={{
        position: 'absolute',
        inset: 0,
        borderRadius: 16,
        overflow: 'hidden',
        background,
        transform: hovered ? 'scale(1.03) translateZ(0)' : 'scale(1) translateZ(0)',
        transition: 'transform 150ms',
      }}
    >
      {logoUrl && (
        <img
          src={logoUrl}
          alt=""
          style={{
            position: 'absolute',
            inset: 0,
            width: '100%',
            height: '100%',
            objectFit: 'cover',
            filter: 'blur(18px)',
            transform: 'scale(1.2) translateZ(0)',
            opacity: 0.6,
          }}
        />
      )}
      <div
        style={{
          position: 'absolute',
          inset: 0,
          borderRadius: 16,
          background: 'linear-gradient(160deg, rgba(255,255,255,.28), rgba(7,6,13,.4))',
          backdropFilter: 'blur(16px) saturate(150%)',
          WebkitBackdropFilter: 'blur(16px) saturate(150%)',
          transform: 'translateZ(0)',
        }}
      />
      <div
        onClick={(e) => {
          e.stopPropagation();
          onToggleFavorite();
        }}
        onMouseEnter={() => setActionHovered(true)}
        onMouseLeave={() => setActionHovered(false)}
        role="button"
        aria-label={isFavorite ? 'Remove favorite' : 'Add favorite'}
        style={{
          position: 'absolute',
          top: 10,
          right: 10,
          width: 30,
          height: 30,
          borderRadius: '50%',
          background: 'rgba(7,6,13,.55)',
          backdropFilter: 'blur(6px)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: '#fff',
          opacity: isFavorite || hovered ? 1 : 0,
          transition: 'opacity 150ms',
          zIndex: 1,
        }}
      >
        {isFavorite ? <IconStarFilled size={16} /> : <IconStar size={16} />}
      </div>
      <span
        style={{
          position: 'absolute',
          top: 14,
          left: 0,
          right: 0,
          textAlign: 'center',
          font: '700 17px "Space Grotesk", sans-serif',
          color: 'rgba(255,255,255,.85)',
          letterSpacing: 0.5,
          textShadow: '0 1px 4px rgba(0,0,0,.5)',
        }}
      >
        {number}
      </span>
      {logoUrl ? (
        <img
          src={logoUrl}
          alt=""
          style={{
            position: 'absolute',
            top: '50%',
            left: '50%',
            transform: 'translate(-50%, -50%)',
            width: '80%',
            aspectRatio: '1',
            objectFit: 'contain',
          }}
        />
      ) : (
        <div
          style={{
            position: 'absolute',
            top: '50%',
            left: '50%',
            transform: 'translate(-50%, -50%)',
            width: '80%',
            aspectRatio: '1',
            borderRadius: 10,
            background: 'rgba(255,255,255,.12)',
          }}
        />
      )}
      <div
        style={{
          position: 'absolute',
          top: '50%',
          left: '50%',
          transform: showPlayButton ? 'translate(-50%, -50%) scale(1)' : 'translate(-50%, -50%) scale(0.85)',
          width: 48,
          height: 48,
          borderRadius: '50%',
          background: 'rgba(7,6,13,.55)',
          backdropFilter: 'blur(6px)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: '#fff',
          opacity: showPlayButton ? 1 : 0,
          transition: 'opacity 150ms, transform 150ms',
          zIndex: 1,
        }}
      >
        <IconPlayerPlayFilled size={20} />
      </div>
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
          position: 'absolute',
          bottom: 10,
          right: 10,
          width: 24,
          height: 24,
          borderRadius: '50%',
          background: 'var(--app-accent)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: 'var(--app-bg)',
          boxShadow: '0 4px 12px rgba(0,0,0,.35)',
          transform: hovered ? 'scale(1.08)' : 'scale(1)',
          transition: 'transform 150ms',
          zIndex: 1,
        }}
      >
        <IconInfoSmall size={48} stroke={2.5} />
      </div>
    </div>
    </div>
    <div
      style={{
        marginTop: 12,
        padding: '0 4px',
        textAlign: 'center',
        font: '600 15px "Space Grotesk", sans-serif',
        color: isPlaying ? 'var(--app-accent)' : 'var(--app-text)',
        lineHeight: 1.3,
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
      }}
    >
      {name}
    </div>
    </div>
  );
}
