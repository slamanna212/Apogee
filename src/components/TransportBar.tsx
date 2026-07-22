import { useRef, useState, type CSSProperties } from 'react';
import { Modal, Popover, Text } from '@mantine/core';
import { IconVolume, IconVolume2, IconVolumeOff } from '@tabler/icons-react';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarStation } from '../types/stellarTunerLog';
import type { PlayerStatus } from '../types/player';
import { CutTypeBadge } from './CutTypeBadge';
import { ChannelActionsMenu } from './ChannelActionsMenu';
import { ChannelArtwork } from './ChannelArtwork';
import { Waveform } from './Waveform';

export type BarMode = 'expanded' | 'collapsed';

interface TransportBarProps {
  mode: BarMode;
  status: PlayerStatus;
  currentChannel: XtreamChannel | null;
  channelMetadata?: StellarChannel;
  nowPlaying?: StellarStation;
  errorMessage?: string | null;
  isBuffering?: boolean;
  volume: number;
  isFavorite?: boolean;
  onToggleFavorite?: () => void;
  onPlus: () => void;
  onMinus: () => void;
  onPlayStop: () => void;
  onVolumeChange: (volume: number) => void;
  muted: boolean;
  onToggleMute: () => void;
  /** True when running as the standalone mini player window (not the full app), which is short and needs the volume popover to open sideways instead of upward. */
  compactVolumePopover?: boolean;
  /** True when running as the standalone mini player window; the artwork expand modal looks wrong in that tiny window, so clicking artwork is disabled there. */
  isMiniPlayer?: boolean;
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
  const size = compact ? 22 : 32;
  const boxWidth = compact ? 19 : 26;
  const btnStyle = {
    width: boxWidth,
    height: size,
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    fontFamily: "'Space Grotesk', sans-serif",
    fontWeight: 700,
    fontSize: compact ? 20 : 28,
    lineHeight: 1,
    color: 'var(--app-dim)',
    cursor: 'pointer',
    userSelect: 'none' as const,
  };
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 0,
        flex: 'none',
        marginRight: compact ? -3 : -6,
      }}
    >
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

function HorizontalVolumeSlider({
  volume,
  onChange,
  width = 90,
}: {
  volume: number;
  onChange: (v: number) => void;
  width?: number;
}) {
  const trackRef = useRef<HTMLDivElement>(null);

  function valueFromClientX(clientX: number) {
    const track = trackRef.current;
    if (!track) return volume;
    const rect = track.getBoundingClientRect();
    const ratio = (clientX - rect.left) / rect.width;
    return Math.max(0, Math.min(100, Math.round(ratio * 100)));
  }

  function handlePointerDown(e: React.PointerEvent<HTMLDivElement>) {
    e.currentTarget.setPointerCapture(e.pointerId);
    onChange(valueFromClientX(e.clientX));
  }
  function handlePointerMove(e: React.PointerEvent<HTMLDivElement>) {
    if (e.buttons !== 1) return;
    onChange(valueFromClientX(e.clientX));
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
        width,
        height: 5,
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
          top: 0,
          left: 0,
          height: '100%',
          width: `${volume}%`,
          borderRadius: 4,
          background: 'var(--app-accent2)',
        }}
      />
      <div
        style={{
          position: 'absolute',
          left: `${volume}%`,
          top: '50%',
          transform: 'translate(-50%, -50%)',
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

function MuteIcon({ muted, onToggleMute }: { muted: boolean; onToggleMute: () => void }) {
  return (
    <span
      onClick={(e) => {
        e.stopPropagation();
        onToggleMute();
      }}
      role="button"
      aria-label={muted ? 'Unmute' : 'Mute'}
      style={{ display: 'flex', cursor: 'pointer', flex: 'none' }}
    >
      {muted ? <IconVolumeOff size={12} style={{ color: 'var(--app-accent)' }} /> : <IconVolume2 size={12} style={{ color: 'var(--app-dim)' }} />}
    </span>
  );
}

function VolumeControl({
  volume,
  onChange,
  compact,
  muted,
  onToggleMute,
}: {
  volume: number;
  onChange: (v: number) => void;
  compact?: boolean;
  muted: boolean;
  onToggleMute: () => void;
}) {
  const [opened, setOpened] = useState(false);

  return (
    <Popover opened={opened} onChange={setOpened} position={compact ? 'left' : 'top'} withArrow shadow="md">
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
      {compact ? (
        <Popover.Dropdown className="apogee-glass" style={{ borderRadius: 20, padding: '0 14px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, height: 36 }}>
            <MuteIcon muted={muted} onToggleMute={onToggleMute} />
            <HorizontalVolumeSlider volume={volume} onChange={onChange} />
            <Text size="xs" fw={600} c="dimmed" style={{ flex: 'none', width: 28 }}>
              {volume}%
            </Text>
          </div>
        </Popover.Dropdown>
      ) : (
        <Popover.Dropdown className="apogee-glass" style={{ borderRadius: 20, padding: '14px 0' }}>
          <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 10, width: 36 }}>
            <Text size="xs" fw={600} c="dimmed">
              {volume}%
            </Text>
            <VerticalVolumeSlider volume={volume} onChange={onChange} />
            <MuteIcon muted={muted} onToggleMute={onToggleMute} />
          </div>
        </Popover.Dropdown>
      )}
    </Popover>
  );
}

const dotsButtonStyle: CSSProperties = {
  width: 42,
  height: 42,
  borderRadius: '50%',
  background: 'var(--app-panel2)',
  border: '1px solid var(--app-border)',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  flex: 'none',
  cursor: 'pointer',
  color: 'var(--app-text)',
};

function BarContent({
  status,
  currentChannel,
  channelMetadata,
  nowPlaying,
  errorMessage,
  onArtworkClick,
}: {
  status: PlayerStatus;
  currentChannel: XtreamChannel | null;
  channelMetadata?: StellarChannel;
  nowPlaying?: StellarStation;
  errorMessage?: string | null;
  onArtworkClick?: (artworkUrl: string) => void;
}) {
  if (!currentChannel) {
    return (
      <>
        <div style={{ width: 65, height: 65, borderRadius: 5, background: 'var(--app-panel2)', flex: 'none' }} />
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
            style={{ width: 65, height: 65, borderRadius: 5, objectFit: 'cover', flex: 'none', background: 'var(--app-panel2)' }}
          />
        ) : (
          <div style={{ width: 65, height: 65, borderRadius: 5, background: 'var(--app-panel2)', flex: 'none' }} />
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
            width: 65,
            height: 65,
            borderRadius: 5,
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

  const artwork = nowPlaying?.artwork_url;
  const title = nowPlaying?.title || currentChannel.name;
  const subtitleParts = [nowPlaying?.artist, nowPlaying?.album].filter(Boolean);

  return (
    <>
      <ChannelArtwork
        channelName={currentChannel.name}
        streamIcon={currentChannel.stream_icon}
        metadata={channelMetadata}
        artworkUrl={artwork}
        size={65}
        radius={5}
        onClick={onArtworkClick && artwork ? () => onArtworkClick(artwork) : undefined}
      />
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
        {nowPlaying?.title && (
          <div
            data-tauri-drag-region
            style={{
              font: '600 10px "Sora", sans-serif',
              color: 'var(--app-dim)',
              textTransform: 'uppercase',
              letterSpacing: '0.06em',
              opacity: 0.7,
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
            }}
          >
            {`CH ${nowPlaying.channel_number} · ${currentChannel.name}`}
          </div>
        )}
      </div>
    </>
  );
}

function CollapsedInfo({
  status,
  currentChannel,
  nowPlaying,
  errorMessage,
  isBuffering,
}: {
  status: PlayerStatus;
  currentChannel: XtreamChannel | null;
  nowPlaying?: StellarStation;
  errorMessage?: string | null;
  isBuffering?: boolean;
}) {
  if (!currentChannel) {
    return (
      <Text data-tauri-drag-region size="xs" c="dimmed" style={{ flex: '1 1 auto', maxWidth: 150, minWidth: 0, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' }}>
        Select a channel
      </Text>
    );
  }

  const title = status === 'error' ? 'Playback error' : nowPlaying?.title || currentChannel.name;
  const subtitle = status === 'loading' ? 'Connecting…' : isBuffering && status === 'playing' ? 'Buffering…' : nowPlaying?.artist;

  return (
    <div data-tauri-drag-region style={{ flex: '1 1 auto', maxWidth: 150, minWidth: 0 }} title={status === 'error' ? errorMessage ?? undefined : undefined}>
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
  channelMetadata,
  nowPlaying,
  errorMessage,
  isBuffering,
  volume,
  isFavorite,
  onToggleFavorite,
  onPlus,
  onMinus,
  onPlayStop,
  onVolumeChange,
  muted,
  onToggleMute,
  compactVolumePopover,
  isMiniPlayer,
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
      portalProps={{ target: '#apogee-window' }}
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
    return (
      <>
        {artworkModal}
        <div
          className="apogee-glass apogee-transport-surface"
          data-tauri-drag-region
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            width: '100%',
            height: '100%',
            boxSizing: 'border-box',
            padding: '0 10px',
            borderRadius: 999,
          }}
        >
          <PlusMinus onPlus={onPlus} onMinus={onMinus} compact />
          <PlayStopButton status={status} onClick={onPlayStop} disabled={!currentChannel} size={36} />
          {currentChannel ? (
            <ChannelArtwork
              channelName={currentChannel.name}
              streamIcon={currentChannel.stream_icon}
              metadata={channelMetadata}
              artworkUrl={nowPlaying?.artwork_url}
              size={36}
              radius={10}
            />
          ) : (
            <div style={{ width: 36, height: 36, borderRadius: 10, background: 'var(--app-panel2)', flex: 'none' }} />
          )}
          <CollapsedInfo status={status} currentChannel={currentChannel} nowPlaying={nowPlaying} errorMessage={errorMessage} isBuffering={isBuffering} />
          <Waveform active={status === 'playing'} bands={4} size="sm" />
        </div>
      </>
    );
  }

  return (
    <>
      {artworkModal}
      <div
        className="apogee-glass apogee-transport-surface"
        data-tauri-drag-region
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 16,
          width: '100%',
          height: '100%',
          boxSizing: 'border-box',
          padding: '0 14px 0 10px',
          borderRadius: 999,
        }}
      >
        <PlusMinus onPlus={onPlus} onMinus={onMinus} />
        <PlayStopButton status={status} onClick={onPlayStop} disabled={!currentChannel} />
        <BarContent
          status={status}
          currentChannel={currentChannel}
          channelMetadata={channelMetadata}
          nowPlaying={nowPlaying}
          errorMessage={errorMessage}
          onArtworkClick={isMiniPlayer ? undefined : setExpandedArtwork}
        />
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, flex: 'none' }}>
          {currentChannel && status !== 'loading' && status !== 'error' && (
            <>
              <CutTypeBadge cutType={nowPlaying?.cut_type} />
              {isBuffering && status === 'playing' && (
                <Text size="xs" c="orange" style={{ flex: 'none' }} title="Buffering">
                  ● Buffering
                </Text>
              )}
              <Waveform active={status === 'playing'} />
            </>
          )}
          <VolumeControl volume={volume} onChange={onVolumeChange} compact={compactVolumePopover} muted={muted} onToggleMute={onToggleMute} />
          {currentChannel && (
            <ChannelActionsMenu
              nowPlaying={nowPlaying}
              isFavorite={!!isFavorite}
              onToggleFavorite={onToggleFavorite ?? (() => {})}
              triggerStyle={{ ...dotsButtonStyle, color: isFavorite ? 'var(--app-accent)' : 'var(--app-text)' }}
              position="top-end"
              layout={isMiniPlayer ? 'horizontal' : 'vertical'}
            />
          )}
        </div>
      </div>
    </>
  );
}
