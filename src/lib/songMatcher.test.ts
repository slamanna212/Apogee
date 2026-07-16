import { describe, expect, it } from 'vitest';
import { normalizeTitle, stripTrailingTitleNumber } from './songMatcher';

describe('stripTrailingTitleNumber', () => {
  it('removes a trailing two-digit marker while preserving case', () => {
    expect(stripTrailingTitleNumber('  Song Name (93)  ')).toBe('Song Name');
    expect(stripTrailingTitleNumber('Song Name(01)')).toBe('Song Name');
  });

  it('preserves other parenthetical title text', () => {
    expect(stripTrailingTitleNumber('Song Name (Live)')).toBe('Song Name (Live)');
    expect(stripTrailingTitleNumber('Song Name (123)')).toBe('Song Name (123)');
  });

  it('is shared by alert title normalization', () => {
    expect(normalizeTitle(' Song   Name (93) ')).toBe('song name');
  });
});
