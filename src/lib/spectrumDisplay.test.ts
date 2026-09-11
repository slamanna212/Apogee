import { describe, expect, it } from 'vitest';
import { spectrumDisplayLevels } from './spectrumDisplay';

describe('spectrumDisplayLevels', () => {
  it('makes small measured differences more visible while preserving their order', () => {
    const input = [0.4, 0.45, 0.5, 0.55, 0.6, 0.55, 0.5, 0.45];
    const output = spectrumDisplayLevels(input, 8);
    expect(output[4] - output[0]).toBeGreaterThan((input[4] - input[0]) * 1.5);
    expect(output.slice(0, 5)).toEqual(output.slice(0, 5).sort((a, b) => a - b));
  });

  it('keeps silence silent and does not manufacture differences in flat input', () => {
    expect(spectrumDisplayLevels(Array(8).fill(0), 8)).toEqual(Array(8).fill(0));
    expect(new Set(spectrumDisplayLevels(Array(8).fill(0.5), 8)).size).toBe(1);
  });

  it('includes every source band in the compact display', () => {
    for (let source = 0; source < 8; source++) {
      const input = Array(8).fill(0);
      input[source] = 1;
      const output = spectrumDisplayLevels(input, 4);
      expect(output[Math.floor(source / 2)]).toBeGreaterThan(0.5);
      expect(output.filter((level) => level > 0)).toHaveLength(1);
    }
  });

  it('handles empty, invalid, and differently sized inputs with bounded heights', () => {
    expect(spectrumDisplayLevels([], 4)).toEqual([0, 0, 0, 0]);
    expect(spectrumDisplayLevels([1], 0)).toEqual([]);
    for (const bands of [3, 8, 12]) {
      const output = spectrumDisplayLevels([NaN, Infinity, -1, 0.5, 2], bands);
      expect(output).toHaveLength(bands);
      for (const level of output) {
        expect(level).toBeGreaterThanOrEqual(0);
        expect(level).toBeLessThanOrEqual(1);
      }
    }
  });
});
