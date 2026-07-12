import { useRef, useState } from 'react';
import { Modal, Popover, Text } from '@mantine/core';
import { IconVolume, IconVolume2 } from '@tabler/icons-react';
import type { XtreamChannel } from '../types/xtream';
import type { StellarStation } from '../types/stellarTunerLog';
import type { PlayerStatus } from '../types/player';
import { CutTypeBadge } from './CutTypeBadge';
import { Waveform } from './Waveform';

export type BarMode = 'expanded' | 'collapsed';

interface TransportBarProps {
  mode: BarMode;
  status: PlayerStatus;
  currentChannel: XtreamChannel | null;
  nowPlaying?: StellarStation;
  errorMessage?: string | null;
  volume: number;
  onPlus: () => void;
  onMinus: () => void;
  onPlayStop: () => void;
  onVolumeChange: (volume: number) => void;
}

function PlusMinus({
  onPlus,
  onMinus,
  compact,
}: {
  onPlus: () => void;
  onMinus: () => void;
  compact?: boolean;
}) {
  const size = compact ? 20 : 30;
  const btnStyle = {
    width: size,
    height: size,
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    fontFamily: "'Space Grotesk', sans-serif",
    fontWeight: 600,
    fontSize: compact ? 12 : 16,
    color: 'var(--app-dim)',
    cursor: 'pointer',
    userSelect: 'none' as const,
  };
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4, flex: 'none' }}>
      <div style={btnStyle} onClick={onPlus} role="button" aria-label="Open browser">
        +
      </div>
      <div style={btnStyle} onClick={onMinus} role="button" aria-label="Collapse">
        &minus;
      </div>
    </div>
  );
}

function PlayStopButton({
  status,
  onClick,
  disabled,
  size = 52,
}: {
  status: PlayerStatus;
  onClick: () => void;
  disabled?: boolean;
  size?: number;
}) {
  const isConnected = status === 'playing' || status === 'loading';
  return (
    <div
      onClick={disabled ? undefined : onClick}
      role="button"
      aria-label={isConnected ? 'Stop' : 'Play'}
      style={{
        width: size,
        height: size,
        borderRadius: '50%',
        background: 'var(--app-accent)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        boxShadow: '0 0 24px var(--app-accent-soft)',
        flex: 'none',
        cursor: disabled ? 'default' : 'pointer',
        opacity: disabled ? 0.5 : 1,
      }}
    >
      {status === 'loading' ? (
        <div
          style={{
            width: size * 0.4,
            height: size * 0.4,
            borderRadius: '50%',
            border: '3px solid var(--app-accent-soft)',
            borderTopColor: 'var(--app-bg)',
            animation: 'spin 0.8s linear infinite',
          }}
        />
      ) : isConnected ? (
        <div style={{ width: size * 0.32, height: size * 0.32, background: 'var(--app-bg)', borderRadius: 3 }} />
      ) : (
        <div
          style={{
            width: 0,
            height: 0,
            borderTop: `${size * 0.24}px solid transparent`,
            borderBottom: `${size * 0.24}px solid transparent`,
            borderLeft: `${size * 0.38}px solid var(--app-bg)`,
            marginLeft: 4,
          }}
        />
      )}
    </div>
  );
}

function VerticalVolumeSlider({
  volume,
  onChange,
  height = 100,
}: {
  volume: number;
  onChange: (v: number) => void;
  height?: number;
}) {
  const trackRef = useRef<HTMLDivElement>(null);

  function valueFromClientY(clientY: number) {
    const track = trackRef.current;
    if (!track) return volume;
    const rect = track.getBoundingClientRect();
    const ratio = 1 - (clientY - rect.top) / rect.height;
    return Math.max(0, Math.min(100, Math.round(ratio * 100)));
  }

  function handlePointerDown(e: React.PointerEvent<HTMLDivElement>) {
    e.currentTarget.setPointerCapture(e.pointerId);
    onChange(valueFromClientY(e.clientY));
  }
  function handlePointerMove(e: React.PointerEvent<HTMLDivElement>) {
    if (e.buttons !== 1) return;
    onChange(valueFromClientY(e.clientY));
  }
  function handleWheel(e: React.WheelEvent<HTMLDivElement>) {
    e.preventDefault();
    onChange(Math.max(0, Math.min(100, volume - Math.sign(e.deltaY) * 3)));
  }

  return (
    <div
      ref={trackRef}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onWheel={handleWheel}
      style={{
        width: 5,
        height,
        borderRadius: 4,
        background: 'rgba(255,255,255,.12)',
        position: 'relative',
        cursor: 'pointer',
        touchAction: 'none',
      }}
    >
      <div
        style={{
          position: 'absolute',
          bottom: 0,
          left: 0,
          width: '100%',
          height: `${volume}%`,
          borderRadius: 4,
          background: 'var(--app-accent2)',
        }}
      />
      <div
        style={{
          position: 'absolute',
          bottom: `${volume}%`,
          left: '50%',
          transform: 'translate(-50%, 50%)',
          width: 14,
          height: 14,
          borderRadius: '50%',
          background: '#fff',
          boxShadow: '0 0 8px var(--app-accent2)',
        }}
      />
    </div>
  );
}

function VolumeControl({ volume, onChange }: { volume: number; onChange: (v: number) => void }) {
  const [opened, setOpened] = useState(false);
  return (
    <Popover opened={opened} onChange={setOpened} position="top" withArrow shadow="md">
      <Popover.Target>
        <div
          onClick={() => setOpened((v) => !v)}
          role="button"
          aria-label="Volume"
          style={{
            width: 42,
            height: 42,
            borderRadius: '50%',
            background: 'var(--app-panel2)',
            border: '1px solid var(--app-border)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            fontSize: 16,
            flex: 'none',
            cursor: 'pointer',
            color: 'var(--app-text)',
          }}
        >
          <IconVolume size={18} />
        </div>
      </Popover.Target>
      <Popover.Dropdown className="apogee-glass" style={{ borderRadius: 20, padding: '14px 0' }}>
        <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 10, width: 36 }}>
          <Text size="xs" fw={600} c="dimmed">
            {volume}%
          </Text>
          <VerticalVolumeSlider volume={volume} onChange={onChange} />
          <IconVolume2 size={12} style={{ color: 'var(--app-dim)' }} />
        </div>
      </Popover.Dropdown>
    </Popover>
  );
}

function BarContent({
  status,
  currentChannel,
  nowPlaying,
  errorMessage,
  onArtworkClick,
}: {
  status: PlayerStatus;
  currentChannel: XtreamChannel | null;
  nowPlaying?: StellarStation;
  errorMessage?: string | null;
  onArtworkClick: (artworkUrl: string) => void;
}) {
  if (!currentChannel) {
    return (
      <>
        <div style={{ width: 56, height: 56, borderRadius: 14, background: 'var(--app-panel2)', flex: 'none' }} />
        <Text data-tauri-drag-region size="sm" c="dimmed" style={{ flex: '1 1 auto', minWidth: 0 }}>
          Select a channel to start listening
        </Text>
      </>
    );
  }

  if (status === 'loading') {
    return (
      <>
        {currentChannel.stream_icon ? (
          <img
            src={currentChannel.stream_icon}
            alt=""
            style={{ width: 56, height: 56, borderRadius: 14, objectFit: 'cover', flex: 'none', background: 'var(--app-panel2)' }}
          />
        ) : (
          <div style={{ width: 56, height: 56, borderRadius: 14, background: 'var(--app-panel2)', flex: 'none' }} />
        )}
        <div data-tauri-drag-region style={{ flex: '1 1 auto', minWidth: 0 }}>
          <div
            data-tauri-drag-region
            style={{
              font: '700 16px "Space Grotesk", sans-serif',
              color: 'var(--app-text)',
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
            }}
          >
            {currentChannel.name}
          </div>
          <div
            data-tauri-drag-region
            style={{
              font: '400 13px "Sora", sans-serif',
              color: 'var(--app-dim)',
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
            }}
          >
            Connecting…
          </div>
        </div>
      </>
    );
  }

  if (status === 'error') {
    return (
      <>
        <div
          style={{
            width: 56,
            height: 56,
            borderRadius: 14,
            background: 'rgba(250,82,82,.12)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: '#ff8787',
            fontWeight: 700,
            flex: 'none',
          }}
        >
          !
        </div>
        <Text data-tauri-drag-region size="sm" c="red.4" style={{ flex: '1 1 auto', minWidth: 0 }}>
          {errorMessage || 'Playback error — tap play to retry'}
        </Text>
      </>
    );
  }

  const artwork = nowPlaying?.artwork_url || currentChannel.stream_icon;
  const title = nowPlaying?.title || currentChannel.name;
  const subtitleParts = [nowPlaying?.artist, nowPlaying?.album].filter(Boolean);

  return (
    <>
      {artwork ? (
        <img
          src={artwork}
          alt=""
          onClick={() => onArtworkClick(artwork)}
          style={{ width: 56, height: 56, borderRadius: 14, objectFit: 'cover', flex: 'none', background: 'var(--app-panel2)', cursor: 'pointer' }}
        />
      ) : (
        <div style={{ width: 56, height: 56, borderRadius: 14, background: 'var(--app-panel2)', flex: 'none' }} />
      )}
      <div data-tauri-drag-region style={{ flex: '1 1 auto', minWidth: 0 }}>
        <div
          data-tauri-drag-region
          style={{
            font: '700 16px "Space Grotesk", sans-serif',
            color: 'var(--app-text)',
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
          }}
        >
          {title}
        </div>
        {subtitleParts.length > 0 && (
          <div
            data-tauri-drag-region
            style={{
              font: '400 13px "Sora", sans-serif',
              color: 'var(--app-dim)',
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
            }}
          >
            {subtitleParts.join(' — ')}
          </div>
        )}
      </div>
      <CutTypeBadge cutType={nowPlaying?.cut_type} />
      <Waveform active={status === 'playing'} />
    </>
  );
}

function CollapsedInfo({
  status,
  currentChannel,
  nowPlaying,
  errorMessage,
}: {
  status: PlayerStatus;
  currentChannel: XtreamChannel | null;
  nowPlaying?: StellarStation;
  errorMessage?: string | null;
}) {
  if (!currentChannel) {
    return (
      <Text data-tauri-drag-region size="xs" c="dimmed" style={{ flex: 'none', maxWidth: 150, minWidth: 0, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' }}>
        Select a channel
      </Text>
    );
  }

  const title = status === 'error' ? 'Playback error' : nowPlaying?.title || currentChannel.name;
  const subtitle = status === 'loading' ? 'Connecting…' : nowPlaying?.artist;

  return (
    <div data-tauri-drag-region style={{ flex: 'none', maxWidth: 150, minWidth: 0 }} title={status === 'error' ? errorMessage ?? undefined : undefined}>
      <div
        data-tauri-drag-region
        style={{
          font: '700 12px "Space Grotesk", sans-serif',
          color: status === 'error' ? '#ff8787' : 'var(--app-text)',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {title}
      </div>
      {subtitle && (
        <div
          data-tauri-drag-region
          style={{
            font: '400 10px "Sora", sans-serif',
            color: 'var(--app-dim)',
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
          }}
        >
          {subtitle}
        </div>
      )}
    </div>
  );
}

export function TransportBar({
  mode,
  status,
  currentChannel,
  nowPlaying,
  errorMessage,
  volume,
  onPlus,
  onMinus,
  onPlayStop,
  onVolumeChange,
}: TransportBarProps) {
  const [expandedArtwork, setExpandedArtwork] = useState<string | null>(null);

  const artworkModal = (
    <Modal
      opened={expandedArtwork !== null}
      onClose={() => setExpandedArtwork(null)}
      withCloseButton={false}
      size="auto"
      radius={26}
      padding={0}
      centered
    >
      {expandedArtwork && (
        <img
          src={expandedArtwork}
          alt=""
          style={{ display: 'block', width: 500, height: 500, maxWidth: '100%', objectFit: 'contain', borderRadius: 26 }}
        />
      )}
    </Modal>
  );

  if (mode === 'collapsed') {
    const artwork = nowPlaying?.artwork_url || currentChannel?.stream_icon;
    return (
      <>
        {artworkModal}
        <div
          className="apogee-glass"
          data-tauri-drag-region
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            height: 56,
            padding: '0 10px',
            borderRadius: 999,
            width: 'fit-content',
            margin: '12px auto',
          }}
        >
          <PlusMinus onPlus={onPlus} onMinus={onMinus} compact />
          <PlayStopButton status={status} onClick={onPlayStop} disabled={!currentChannel} size={36} />
          <div style={{ width: 36, height: 36, borderRadius: 10, background: 'var(--app-panel2)', flex: 'none', overflow: 'hidden' }}>
            {artwork && (
              <img
                src={artwork}
                alt=""
                onClick={() => setExpandedArtwork(artwork)}
                style={{ width: '100%', height: '100%', objectFit: 'cover', cursor: 'pointer' }}
              />
            )}
          </div>
          <CollapsedInfo status={status} currentChannel={currentChannel} nowPlaying={nowPlaying} errorMessage={errorMessage} />
          <Waveform active={status === 'playing'} bands={4} size="sm" />
        </div>
      </>
    );
  }

  return (
    <>
      {artworkModal}
      <div
        className="apogee-glass"
        data-tauri-drag-region
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 16,
          height: 84,
          padding: '0 14px 0 10px',
          borderRadius: 999,
          margin: 8,
        }}
      >
        <PlusMinus onPlus={onPlus} onMinus={onMinus} />
        <PlayStopButton status={status} onClick={onPlayStop} disabled={!currentChannel} />
        <BarContent
          status={status}
          currentChannel={currentChannel}
          nowPlaying={nowPlaying}
          errorMessage={errorMessage}
          onArtworkClick={setExpandedArtwork}
        />
        <VolumeControl volume={volume} onChange={onVolumeChange} />
      </div>
    </>
  );
}
