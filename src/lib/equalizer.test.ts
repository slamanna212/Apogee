import { describe, expect, it } from 'vitest';
import {
  DEFAULT_EQUALIZER,
  detectEqualizerPreset,
  EQUALIZER_PRESETS,
  normalizeEqualizerSettings,
} from './equalizer';

describe('equalizer presets', () => {
  it('detects built-in curves and treats edited curves as custom', () => {
    expect(detectEqualizerPreset(EQUALIZER_PRESETS.rock)).toBe('rock');
    expect(detectEqualizerPreset([4, 3, 1, -1, -2, 1, 3, 4, 4, 2])).toBe('custom');
  });
});

describe('normalizeEqualizerSettings', () => {
  it('defaults missing settings for existing installations', () => {
    expect(normalizeEqualizerSettings(undefined)).toEqual(DEFAULT_EQUALIZER);
  });

  it('clamps gains and derives the preset from the actual curve', () => {
    expect(normalizeEqualizerSettings({
      enabled: true,
      preset: 'rock',
      gains: [20, -20, 0, 0, 0, 0, 0, 0, 0, 0],
    })).toEqual({
      enabled: true,
      preset: 'custom',
      gains: [12, -12, 0, 0, 0, 0, 0, 0, 0, 0],
    });
  });

  it('replaces malformed band arrays with a safe flat curve', () => {
    expect(normalizeEqualizerSettings({ enabled: true, gains: [1, 2] })).toEqual({
      enabled: true,
      preset: 'flat',
      gains: Array(10).fill(0),
    });
  });
});
