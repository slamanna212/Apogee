import { forwardRef, useImperativeHandle, useState } from 'react';
import { Slider, Switch } from '@mantine/core';
import { OptionRow } from './OptionRow';
import { TransportBar } from '../TransportBar';
import { useSettingsStore, DEFAULT_RAIL_SETTINGS, type RailColorSource, type Settings } from '../../stores/settingsStore';
import type { XtreamChannel } from '../../types/xtream';
import type { StellarStation } from '../../types/stellarTunerLog';
import type { SettingsPanelProps, SettingsResetHandle } from './types';

type RailToggleKey = 'railShowArtwork' | 'railShowVisualizer' | 'railShowCutType' | 'railShowBitrate' | 'railShowChannelLine' | 'railScrollTitles';

const SWITCH_ROWS: { key: RailToggleKey; label: string; description: string }[] = [
  { key: 'railShowArtwork', label: 'Album artwork', description: 'The art tile left of the track name' },
  { key: 'railShowVisualizer', label: 'Visualizer', description: 'Live spectrum bars while audio is playing' },
  { key: 'railShowCutType', label: 'Media type', description: 'Song, Talk, Sports or Program for the current cut' },
  { key: 'railShowBitrate', label: 'Bitrate', description: 'Shows the stream bitrate next to the media type' },
  { key: 'railShowChannelLine', label: 'Channel line', description: 'The small channel number and name under the artist' },
  { key: 'railScrollTitles', label: 'Scroll long titles', description: 'Long track names and artist lines scroll once instead of being cut off' },
];

const COLOR_SOURCE_OPTIONS: { value: RailColorSource; label: string }[] = [
  { value: 'artwork', label: 'From album art' },
  { value: 'static', label: 'Static accent' },
];

interface SampleArt {
  bg: [string, string];
  shape: string;
  mode: 'orb' | 'bars' | 'diag';
}

interface SampleTrack {
  title: string;
  artist: string;
  album: string;
  channelName: string;
  channelNumber: number;
  bitrateKbps: number;
  art: SampleArt;
}

/** Fictional tracks whose art is drawn locally, so the preview can show art-derived color
 *  changing without depending on network artwork or on anything being played. */
const SAMPLE_TRACKS: SampleTrack[] = [
  {
    title: 'Midnight Interchange (Extended Late Night Mix)',
    artist: 'Vela Tomas',
    album: 'Nocturne Transmissions',
    channelName: 'Deep Tracks',
    channelNumber: 34,
    bitrateKbps: 256,
    art: { bg: ['#ff8a4c', '#7a1f4b'], shape: '#ffd4a3', mode: 'orb' },
  },
  {
    title: 'Coastline',
    artist: 'The Harbor Lights',
    album: 'Salt & Signal',
    channelName: 'Coastal FM',
    channelNumber: 12,
    bitrateKbps: 128,
    art: { bg: ['#0b4f5e', '#5ff0e2'], shape: '#04222a', mode: 'bars' },
  },
  {
    title: 'Somewhere Between the Static and the Signal, Pt. II',
    artist: 'Junia Ford & The Long Players',
    album: 'Broadcast Hours',
    channelName: 'Night Shift',
    channelNumber: 7,
    bitrateKbps: 256,
    art: { bg: ['#2b1b6b', '#d84fa8'], shape: '#f6d24a', mode: 'diag' },
  },
];

const artCache = new Map<SampleArt, string>();

function sampleArtUrl(art: SampleArt): string {
  const cached = artCache.get(art);
  if (cached) return cached;
  const s = 120;
  const canvas = document.createElement('canvas');
  canvas.width = s;
  canvas.height = s;
  const ctx = canvas.getContext('2d');
  if (!ctx) return '';
  const gradient = ctx.createLinearGradient(0, 0, s, s);
  gradient.addColorStop(0, art.bg[0]);
  gradient.addColorStop(1, art.bg[1]);
  ctx.fillStyle = gradient;
  ctx.fillRect(0, 0, s, s);
  ctx.fillStyle = art.shape;
  if (art.mode === 'orb') {
    ctx.beginPath();
    ctx.arc(s * 0.62, s * 0.38, s * 0.22, 0, Math.PI * 2);
    ctx.fill();
  } else if (art.mode === 'bars') {
    for (let i = 0; i < 5; i++) ctx.fillRect(s * (0.12 + i * 0.17), s * (0.62 - i * 0.07), s * 0.08, s * 0.3);
  } else {
    ctx.translate(s * 0.5, s * 0.5);
    ctx.rotate(Math.PI / 5);
    ctx.fillRect(-s * 0.42, -s * 0.07, s * 0.84, s * 0.14);
  }
  const url = canvas.toDataURL('image/png');
  artCache.set(art, url);
  return url;
}

function previewProps(track: SampleTrack): { channel: XtreamChannel; station: StellarStation } {
  return {
    channel: { stream_id: -1, name: track.channelName, stream_icon: '', num: track.channelNumber, category_id: '' },
    station: {
      id: 'preview',
      name: track.channelName,
      channel_number: track.channelNumber,
      artist: track.artist,
      title: track.title,
      album: track.album,
      cut_type: 'song',
      artwork_url: sampleArtUrl(track.art),
      itunes_id: '',
    },
  };
}

const noop = () => {};

const labelStyle = { font: "600 13px 'Sora', sans-serif" } as const;
const divider = <div style={{ height: 1, background: 'var(--app-border)' }} />;

export const PlaybackBarPanel = forwardRef<SettingsResetHandle, SettingsPanelProps>(function PlaybackBarPanel({ onSaved }, ref) {
  const settings = useSettingsStore((s) => s.settings);
  const updateSettings = useSettingsStore((s) => s.update);
  const [trackIndex, setTrackIndex] = useState(0);
  // Held locally while dragging so the settings file is written once, on release.
  const [periodDraft, setPeriodDraft] = useState<number | null>(null);
  const period = periodDraft ?? settings.railScrollPeriodSeconds;

  useImperativeHandle(ref, () => ({
    async reset() {
      await updateSettings(DEFAULT_RAIL_SETTINGS);
    },
  }));

  function save(patch: Partial<Settings>) {
    void updateSettings(patch);
    onSaved();
  }

  const track = SAMPLE_TRACKS[trackIndex];
  const { channel, station } = previewProps(track);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 26 }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        <label style={labelStyle}>Preview</label>
        <div
          style={{
            padding: 20,
            borderRadius: 18,
            background: 'rgba(255,255,255,.03)',
            border: '1px solid var(--app-border)',
            display: 'flex',
            overflowX: 'auto',
          }}
        >
          {/* The real rail, non-interactive, reading the same settings it will use live. */}
          <div aria-hidden style={{ width: 560, height: 84, flex: 'none', margin: '0 auto', pointerEvents: 'none' }}>
            <TransportBar
              mode="expanded"
              status="playing"
              currentChannel={channel}
              nowPlaying={station}
              volume={80}
              muted={false}
              bitrateKbps={track.bitrateKbps}
              onPlus={noop}
              onMinus={noop}
              onPlayStop={noop}
              onVolumeChange={noop}
              onToggleMute={noop}
              isMiniPlayer
            />
          </div>
        </div>
        <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <span style={{ fontSize: 11.5, color: 'var(--app-dim2)' }}>Preview track</span>
          {SAMPLE_TRACKS.map((t, i) => (
            <img
              key={t.title}
              src={sampleArtUrl(t.art)}
              alt={t.title}
              title={t.title}
              role="button"
              aria-pressed={i === trackIndex}
              onClick={() => setTrackIndex(i)}
              style={{
                width: 26,
                height: 26,
                borderRadius: 6,
                cursor: 'pointer',
                display: 'block',
                outline: i === trackIndex ? '2px solid var(--app-accent)' : 'none',
                outlineOffset: 2,
              }}
            />
          ))}
        </div>
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <label style={labelStyle}>Accent color</label>
        <div style={{ fontSize: 12, color: 'var(--app-dim)' }}>Tints the play button, its glow, and the visualizer</div>
        <div style={{ marginTop: 4 }}>
          <OptionRow
            options={COLOR_SOURCE_OPTIONS}
            value={settings.railColorSource}
            onChange={(value) => save({ railColorSource: value })}
          />
        </div>
      </div>

      {divider}

      <div style={{ display: 'flex', flexDirection: 'column', gap: 20 }}>
        {SWITCH_ROWS.map((row) => (
          <Switch
            key={row.key}
            label={row.label}
            description={row.description}
            checked={settings[row.key]}
            onChange={(e) => save({ [row.key]: e.currentTarget.checked })}
          />
        ))}
      </div>

      {divider}

      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <label style={{ ...labelStyle, opacity: settings.railScrollTitles ? 1 : 0.5 }}>Scroll a long title every</label>
        <div style={{ display: 'flex', alignItems: 'center', gap: 16, maxWidth: 420, paddingBottom: 18 }}>
          <Slider
            style={{ flex: 1 }}
            min={4}
            max={16}
            step={1}
            marks={[4, 8, 12, 16].map((value) => ({ value, label: `${value}s` }))}
            label={null}
            disabled={!settings.railScrollTitles}
            value={period}
            onChange={setPeriodDraft}
            onChangeEnd={(value) => {
              setPeriodDraft(null);
              save({ railScrollPeriodSeconds: value });
            }}
          />
          <span style={{ font: "600 13px 'Space Grotesk', sans-serif", width: 28, flex: 'none', opacity: settings.railScrollTitles ? 1 : 0.5 }}>
            {period}s
          </span>
        </div>
        <div style={{ fontSize: 12, color: 'var(--app-dim)' }}>Only text too long for the rail scrolls; everything else stays still.</div>
      </div>
    </div>
  );
});
