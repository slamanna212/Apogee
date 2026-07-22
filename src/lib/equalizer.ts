export const EQUALIZER_BANDS = [31, 62, 125, 250, 500, 1_000, 2_000, 4_000, 8_000, 16_000] as const;

export type EqualizerPreset = 'flat' | 'bass-boost' | 'treble-boost' | 'vocal' | 'rock' | 'pop' | 'custom';

export interface EqualizerSettings {
  enabled: boolean;
  preset: EqualizerPreset;
  gains: number[];
}

export const EQUALIZER_PRESETS: Record<Exclude<EqualizerPreset, 'custom'>, readonly number[]> = {
  flat: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  'bass-boost': [6, 5, 4, 2, 0, -1, -1, 0, 0, 0],
  'treble-boost': [0, 0, -1, -1, 0, 1, 3, 5, 6, 6],
  vocal: [-2, -2, -1, 0, 2, 4, 4, 2, 0, -1],
  rock: [4, 3, 1, -1, -2, 1, 3, 4, 4, 3],
  pop: [-1, 1, 3, 4, 2, 0, -1, -1, 1, 2],
};

export const DEFAULT_EQUALIZER: EqualizerSettings = {
  enabled: false,
  preset: 'flat',
  gains: [...EQUALIZER_PRESETS.flat],
};

const PRESET_IDS = Object.keys(EQUALIZER_PRESETS) as Exclude<EqualizerPreset, 'custom'>[];

export function detectEqualizerPreset(gains: readonly number[]): EqualizerPreset {
  return PRESET_IDS.find((preset) => EQUALIZER_PRESETS[preset].every((gain, index) => gain === gains[index])) ?? 'custom';
}

export function normalizeEqualizerSettings(value: unknown): EqualizerSettings {
  if (!value || typeof value !== 'object') return { ...DEFAULT_EQUALIZER, gains: [...DEFAULT_EQUALIZER.gains] };

  const candidate = value as Partial<EqualizerSettings>;
  const gains = Array.isArray(candidate.gains) && candidate.gains.length === EQUALIZER_BANDS.length
    ? candidate.gains.map((gain) => typeof gain === 'number' && Number.isFinite(gain) ? Math.max(-12, Math.min(12, gain)) : 0)
    : [...DEFAULT_EQUALIZER.gains];

  return {
    enabled: candidate.enabled === true,
    preset: detectEqualizerPreset(gains),
    gains,
  };
}

export function formatEqualizerBand(frequency: number): string {
  return frequency >= 1_000 ? `${frequency / 1_000}k` : String(frequency);
}
