import { useEffect, useState, type CSSProperties } from 'react';
import { useComputedColorScheme } from '@mantine/core';
import type { StellarChannel } from '../types/stellarTunerLog';
import { pickChannelLogoUrl } from '../lib/channelLogo';
import { hashGradient } from './ChannelCard';

interface ChannelArtworkProps {
  channelName: string;
  streamIcon?: string;
  metadata?: StellarChannel;
  /** Track-level art (e.g. StellarTunerLog `artwork_url`). Falls back to the channel logo when absent or broken. */
  artworkUrl?: string;
  size: number;
  radius: number;
  onClick?: () => void;
  style?: CSSProperties;
}

/**
 * Track art (`artworkUrl`) is often missing or 404s. Rather than leave a bare
 * empty square, fall back to the same colored channel logo tile used on the
 * browse cards, so there's always something on screen.
 */
export function ChannelArtwork({ channelName, streamIcon, metadata, artworkUrl, size, radius, onClick, style }: ChannelArtworkProps) {
  const colorScheme = useComputedColorScheme('dark');
  const [artFailed, setArtFailed] = useState(false);
  const [logoFailed, setLogoFailed] = useState(false);

  useEffect(() => setArtFailed(false), [artworkUrl]);

  const logoUrl = pickChannelLogoUrl(metadata?.logos, colorScheme) || streamIcon;
  useEffect(() => setLogoFailed(false), [logoUrl]);

  if (artworkUrl && !artFailed) {
    return (
      <img
        src={artworkUrl}
        alt=""
        onClick={onClick}
        onError={() => setArtFailed(true)}
        style={{
          width: size,
          height: size,
          borderRadius: radius,
          objectFit: 'cover',
          flex: 'none',
          background: 'var(--app-panel2)',
          cursor: onClick ? 'pointer' : 'default',
          transform: 'translateZ(0)',
          ...style,
        }}
      />
    );
  }

  const background = metadata?.dark_bg_color || hashGradient(channelName);

  return (
    <div
      onClick={onClick}
      style={{
        width: size,
        height: size,
        borderRadius: radius,
        background,
        flex: 'none',
        overflow: 'hidden',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        cursor: onClick ? 'pointer' : 'default',
        transform: 'translateZ(0)',
        ...style,
      }}
    >
      {logoUrl && !logoFailed && (
        <img
          src={logoUrl}
          alt=""
          onError={() => setLogoFailed(true)}
          style={{ width: '65%', height: '65%', objectFit: 'contain' }}
        />
      )}
    </div>
  );
}
