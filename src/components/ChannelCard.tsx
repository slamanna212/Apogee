import { useState } from 'react';
import { IconPlayerPlayFilled, IconStar, IconStarFilled } from '@tabler/icons-react';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel } from '../types/stellarTunerLog';

const GRADIENTS: [string, string][] = [
  ['#ff7a5c', '#8b6bff'],
  ['#45e0d8', '#5c7cfa'],
  ['#f76707', '#f06595'],
  ['#12b886', '#339af0'],
  ['#5c7cfa', '#8b6bff'],
  ['#9775fa', '#f06595'],
  ['#fcc419', '#f76707'],
  ['#339af0', '#12b886'],
];

function hashGradient(seed: string): string {
  let hash = 0;
  for (let i = 0; i < seed.length; i++) hash = (hash * 31 + seed.charCodeAt(i)) | 0;
  const [a, b] = GRADIENTS[Math.abs(hash) % GRADIENTS.length];
  return `linear-gradient(150deg, ${a}, ${b})`;
}

interface ChannelCardProps {
  channel: XtreamChannel;
  metadata?: StellarChannel;
  isFavorite: boolean;
  onToggleFavorite: () => void;
  onClick: () => void;
  onPlay: () => void;
}

export function ChannelCard({ channel, metadata, isFavorite, onToggleFavorite, onClick, onPlay }: ChannelCardProps) {
  const [hovered, setHovered] = useState(false);
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
        background,
        transform: hovered ? 'scale(1.03)' : 'scale(1)',
        boxShadow: hovered ? '0 8px 24px var(--app-accent-soft)' : 'none',
        transition: 'transform 150ms, box-shadow 150ms',
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
            transform: 'scale(1.2)',
            opacity: 0.6,
          }}
        />
      )}
      <div
        style={{
          position: 'absolute',
          inset: 0,
          background: 'linear-gradient(160deg, rgba(255,255,255,.28), rgba(7,6,13,.4))',
          backdropFilter: 'blur(16px) saturate(150%)',
          WebkitBackdropFilter: 'blur(16px) saturate(150%)',
        }}
      />
      <div
        onClick={(e) => {
          e.stopPropagation();
          onToggleFavorite();
        }}
        role="button"
        aria-label={isFavorite ? 'Remove favorite' : 'Add favorite'}
        style={{
          position: 'absolute',
          top: 8,
          right: 8,
          width: 26,
          height: 26,
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
        {isFavorite ? <IconStarFilled size={14} /> : <IconStar size={14} />}
      </div>
      <span
        style={{
          position: 'absolute',
          top: 12,
          left: 0,
          right: 0,
          textAlign: 'center',
          font: '700 15px "Space Grotesk", sans-serif',
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
        onClick={(e) => {
          e.stopPropagation();
          onPlay();
        }}
        role="button"
        aria-label={`Play ${name}`}
        style={{
          position: 'absolute',
          bottom: 8,
          right: 8,
          width: 34,
          height: 34,
          borderRadius: '50%',
          background: 'var(--app-accent)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: '#07060d',
          boxShadow: '0 4px 12px rgba(0,0,0,.35)',
          transform: hovered ? 'scale(1.08)' : 'scale(1)',
          transition: 'transform 150ms',
          zIndex: 1,
        }}
      >
        <IconPlayerPlayFilled size={16} />
      </div>
    </div>
    <div
      style={{
        marginTop: 10,
        padding: '0 4px',
        textAlign: 'center',
        font: '600 13.5px "Space Grotesk", sans-serif',
        color: 'var(--app-text)',
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
