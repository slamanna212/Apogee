import { DEFAULT_AUDIO_BUFFER, audioBufferError, type AudioBufferSettings } from '../../lib/audioBuffer';
import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
import { Alert, Button, Group, Loader, NumberInput, Select, Slider, Switch, Text } from '@mantine/core';
import { OptionRow } from './OptionRow';
import { useSettingsStore } from '../../stores/settingsStore';
import { usePlayerStore } from '../../stores/playerStore';
import { listDevices, setEqualizer as playerSetEqualizer, type DeviceDescriptor } from '../../lib/playerClient';
import {
  detectEqualizerPreset,
  DEFAULT_EQUALIZER,
  EQUALIZER_BANDS,
  EQUALIZER_PRESETS,
  formatEqualizerBand,
  type EqualizerPreset,
  type EqualizerSettings,
} from '../../lib/equalizer';
import type { SettingsPanelProps, SettingsResetHandle } from './types';

const EQUALIZER_PRESET_OPTIONS: { value: Exclude<EqualizerPreset, 'custom'>; label: string }[] = [
  { value: 'flat', label: 'Flat' },
  { value: 'bass-boost', label: 'Bass Boost' },
  { value: 'treble-boost', label: 'Treble Boost' },
  { value: 'vocal', label: 'Vocal' },
  { value: 'rock', label: 'Rock' },
  { value: 'pop', label: 'Pop' },
  { value: 'edm', label: 'EDM' },
  { value: 'country', label: 'Country' },
];

export const AudioPanel = forwardRef<SettingsResetHandle, SettingsPanelProps>(function AudioPanel({ onSaved }, ref) {
  const settings = useSettingsStore((s) => s.settings);
  const updateSettings = useSettingsStore((s) => s.update);
  const deviceMigrationNotice = useSettingsStore((s) => s.deviceMigrationNotice);
  const dismissDeviceMigrationNotice = useSettingsStore((s) => s.dismissDeviceMigrationNotice);
  const hasSelectedChannel = usePlayerStore((s) => s.currentChannel !== null);

  const [audioDevices, setAudioDevices] = useState<DeviceDescriptor[]>([]);
  const [loadingAudioDevices, setLoadingAudioDevices] = useState(false);
  const [audioDeviceError, setAudioDeviceError] = useState<string | null>(null);
  const [equalizer, setEqualizerState] = useState<EqualizerSettings>(settings.equalizer);
  const [equalizerError, setEqualizerError] = useState<string | null>(null);
  const [bufferDraft, setBufferDraft] = useState<Record<keyof AudioBufferSettings, number | string>>(settings.audioBuffer);
  const [bufferError, setBufferError] = useState<string | null>(null);
  const [savingBuffer, setSavingBuffer] = useState(false);
  useEffect(() => { setBufferDraft(settings.audioBuffer); }, [settings.audioBuffer]);
  const bufferValues = bufferDraft as AudioBufferSettings;
  const bufferValidation = audioBufferError(bufferValues);
  const bufferChanged = (Object.keys(settings.audioBuffer) as (keyof AudioBufferSettings)[])
    .some((key) => bufferDraft[key] !== settings.audioBuffer[key]);

  async function saveBuffer(next: AudioBufferSettings) {
    setSavingBuffer(true);
    setBufferError(null);
    try {
      await updateSettings({ audioBuffer: next });
      onSaved();
    } catch (err) {
      setBufferError(err instanceof Error ? err.message : String(err));
    } finally {
      setSavingBuffer(false);
    }
  }

  const equalizerRef = useRef(equalizer);
  const equalizerPreviewTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingEqualizerPreview = useRef<EqualizerSettings | null>(null);

  useEffect(() => {
    equalizerRef.current = settings.equalizer;
    setEqualizerState(settings.equalizer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings.equalizer]);

  useEffect(() => () => {
    if (equalizerPreviewTimer.current) clearTimeout(equalizerPreviewTimer.current);
  }, []);

  function applyEqualizer(next: EqualizerSettings) {
    if (!hasSelectedChannel) return;
    setEqualizerError(null);
    playerSetEqualizer(next.enabled, next.gains).catch((err) => {
      setEqualizerError(err instanceof Error ? err.message : String(err));
    });
  }

  function previewEqualizer(next: EqualizerSettings) {
    pendingEqualizerPreview.current = next;
    if (equalizerPreviewTimer.current) return;
    equalizerPreviewTimer.current = setTimeout(() => {
      equalizerPreviewTimer.current = null;
      const pending = pendingEqualizerPreview.current;
      pendingEqualizerPreview.current = null;
      if (pending) applyEqualizer(pending);
    }, 50);
  }

  function replaceEqualizer(next: EqualizerSettings, persist: boolean, preview = true) {
    if (equalizerPreviewTimer.current) {
      clearTimeout(equalizerPreviewTimer.current);
      equalizerPreviewTimer.current = null;
      pendingEqualizerPreview.current = null;
    }
    equalizerRef.current = next;
    setEqualizerState(next);
    if (preview) applyEqualizer(next);
    if (persist) void updateSettings({ equalizer: next }).then(onSaved);
  }

  function handleEqualizerPreset(value: Exclude<EqualizerPreset, 'custom'>) {
    replaceEqualizer(
      { enabled: equalizerRef.current.enabled, preset: value, gains: [...EQUALIZER_PRESETS[value]] },
      true,
    );
  }

  function handleEqualizerBand(index: number, gain: number, persist: boolean) {
    const gains = [...equalizerRef.current.gains];
    gains[index] = gain;
    const next = { ...equalizerRef.current, preset: detectEqualizerPreset(gains), gains };
    equalizerRef.current = next;
    setEqualizerState(next);
    if (persist) {
      void updateSettings({ equalizer: next }).then(onSaved);
    } else {
      previewEqualizer(next);
    }
  }

  // Only enumerate when the picker is actually opened rather than on every Settings
  // visit. The explicit "System default" option below covers following the OS
  // default, so device descriptors are listed as-is.
  async function loadAudioDevices() {
    if (loadingAudioDevices) return;
    setLoadingAudioDevices(true);
    try {
      setAudioDevices(await listDevices());
    } catch {
      // Leave the list as-is; the stored selection (if any) still shows.
    } finally {
      setLoadingAudioDevices(false);
    }
  }

  async function handleAudioDeviceChange(value: string | null) {
    setAudioDeviceError(null);
    const device = audioDevices.find((d) => d.id === value);
    const selection = value ? { id: value, name: device?.name ?? value } : null;
    try {
      await updateSettings({ audioDevice: selection });
      onSaved();
    } catch (err) {
      setAudioDeviceError(err instanceof Error ? err.message : String(err));
    }
  }

  useImperativeHandle(ref, () => ({
    async reset() {
      setAudioDeviceError(null);
      setBufferError(null);
      replaceEqualizer(DEFAULT_EQUALIZER, false);
      await updateSettings({
        audioDevice: null,
        equalizer: DEFAULT_EQUALIZER,
        audioBuffer: { ...DEFAULT_AUDIO_BUFFER },
      });
    },
  }));

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
      {deviceMigrationNotice && (
        <Alert color="yellow" title="Output device reset to system default" withCloseButton onClose={dismissDeviceMigrationNotice}>
          {deviceMigrationNotice}
        </Alert>
      )}
      {audioDeviceError && (
        <Alert color="red" title="Could not switch output device" withCloseButton onClose={() => setAudioDeviceError(null)}>
          {audioDeviceError}
        </Alert>
      )}
      <Select
        label="Output device"
        description="Where audio plays, and the source the visualizer listens to"
        placeholder="System default"
        data={[
          { value: '', label: 'System default' },
          // Surface the saved device even before the list has loaded so it shows
          // its label instead of a bare id on first open.
          ...(settings.audioDevice && !audioDevices.some((d) => d.id === settings.audioDevice!.id)
            ? [{ value: settings.audioDevice.id, label: settings.audioDevice.name }]
            : []),
          ...audioDevices.map((d) => ({ value: d.id, label: d.isDefault ? `${d.name} (system default)` : d.name })),
        ]}
        value={settings.audioDevice?.id ?? ''}
        onChange={handleAudioDeviceChange}
        onDropdownOpen={() => { if (audioDevices.length === 0) void loadAudioDevices(); }}
        rightSection={loadingAudioDevices ? <Loader size="xs" /> : undefined}
        allowDeselect={false}
        searchable
      />
      <div style={{ height: 1, background: 'var(--app-border)' }} />
      <Group justify="space-between" align="center" wrap="nowrap">
        <Switch
          label="Equalizer"
          description="Shape the sound across ten frequency bands"
          checked={equalizer.enabled}
          onChange={(event) => replaceEqualizer({ ...equalizerRef.current, enabled: event.currentTarget.checked }, true)}
        />
        <Button
          variant="subtle"
          size="compact-sm"
          disabled={equalizer.preset === 'flat'}
          onClick={() => handleEqualizerPreset('flat')}
        >
          Reset
        </Button>
      </Group>
      <OptionRow
        shape="pill"
        options={[
          ...EQUALIZER_PRESET_OPTIONS,
          ...(equalizer.preset === 'custom' ? [{ value: 'custom' as const, label: 'Custom' }] : []),
        ]}
        value={equalizer.preset === 'custom' ? 'custom' : equalizer.preset}
        onChange={(v) => { if (v !== 'custom') handleEqualizerPreset(v); }}
        disabled={!equalizer.enabled}
      />
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(10, minmax(34px, 1fr))',
          gap: 8,
          overflowX: 'auto',
          padding: '8px 2px 2px',
        }}
      >
        {EQUALIZER_BANDS.map((frequency, index) => (
          <div key={frequency} style={{ display: 'flex', minWidth: 34, flexDirection: 'column', alignItems: 'center', gap: 8 }}>
            <Text size="xs" c="dimmed" style={{ fontVariantNumeric: 'tabular-nums' }}>
              {equalizer.gains[index] > 0 ? '+' : ''}
              {equalizer.gains[index]}
            </Text>
            <Slider
              orientation="vertical"
              h={150}
              min={-12}
              max={12}
              step={1}
              value={equalizer.gains[index]}
              onChange={(gain) => handleEqualizerBand(index, gain, false)}
              onChangeEnd={(gain) => handleEqualizerBand(index, gain, true)}
              disabled={!equalizer.enabled}
              label={(gain) => `${gain > 0 ? '+' : ''}${gain} dB`}
              aria-label={`${formatEqualizerBand(frequency)}Hz gain`}
            />
            <Text size="xs" fw={600}>{formatEqualizerBand(frequency)}</Text>
          </div>
        ))}
      </div>
      <Text size="xs" c="dimmed">
        Frequencies are in Hz. Apogee automatically adds headroom when bands are boosted to help prevent clipping.
      </Text>
      {equalizerError && (
        <Alert color="yellow" title="Couldn't apply the equalizer">
          Playback will continue unchanged. {equalizerError}
        </Alert>
      )}
      <div style={{ height: 1, background: 'var(--app-border)' }} />
      <Group justify="space-between" align="center">
        <Text fw={600}>Audio buffering</Text>
        <Button variant="subtle" size="compact-sm" disabled={savingBuffer}
          onClick={() => { void saveBuffer({ ...DEFAULT_AUDIO_BUFFER }); }}>
          Reset
        </Button>
      </Group>
      <Text size="sm" c="dimmed">
        Advanced settings. Defaults work well for most connections. Larger buffers can help
        with interruptions but may increase playback delay. Changes apply the next time playback starts.
      </Text>
      <NumberInput
        label="Buffer capacity"
        description="Maximum audio held in reserve (100–10,000 ms)."
        value={bufferDraft.capacityMs}
        onChange={(capacityMs) => setBufferDraft((draft) => ({ ...draft, capacityMs }))}
        min={100} max={10000} step={50} allowDecimal={false} allowNegative={false}
        suffix=" ms" disabled={savingBuffer}
      />
      <NumberInput
        label="Startup buffer"
        description="Audio needed before starting or resuming playback. Must fit within capacity."
        value={bufferDraft.startMs}
        onChange={(startMs) => setBufferDraft((draft) => ({ ...draft, startMs }))}
        min={50} max={10000} step={50} allowDecimal={false} allowNegative={false}
        suffix=" ms" disabled={savingBuffer}
      />
      <NumberInput
        label="Rebuffer threshold"
        description="Pause to refill when audio drops to this level. Must be below the startup buffer."
        value={bufferDraft.rebufferMs}
        onChange={(rebufferMs) => setBufferDraft((draft) => ({ ...draft, rebufferMs }))}
        min={0} max={9999} step={50} allowDecimal={false} allowNegative={false}
        suffix=" ms" disabled={savingBuffer}
      />
      {bufferValidation && <Text size="sm" c="red" role="alert">{bufferValidation}</Text>}
      {bufferError && <Alert color="red" title="Could not save audio buffering">{bufferError}</Alert>}
      <Group justify="flex-end">
        <Button disabled={(!bufferChanged && !bufferError) || !!bufferValidation} loading={savingBuffer}
          onClick={() => { void saveBuffer(bufferValues); }}>
          Save buffering
        </Button>
      </Group>
    </div>
  );
});
