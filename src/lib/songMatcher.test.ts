import { describe, expect, it } from 'vitest';
import type { StellarStation } from '../types/stellarTunerLog';
import type { AlertEntry } from '../types/alerts';
import { matchesEntry, normalizeTitle, stripTrailingTitleNumber } from './songMatcher';

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

  it('treats null or missing remote titles as empty strings', () => {
    expect(stripTrailingTitleNumber(null)).toBe('');
    expect(normalizeTitle(undefined)).toBe('');
  });
});

function station(overrides: Partial<StellarStation> = {}): StellarStation {
  return {
    id: 'station',
    name: 'Channel',
    channel_number: 1,
    artist: 'Artist',
    title: 'Song Name',
    album: 'Album',
    cut_type: 'Song',
    artwork_url: '',
    itunes_id: '',
    ...overrides,
  };
}

function entry(overrides: Partial<AlertEntry> = {}): AlertEntry {
  return { id: 'entry', type: 'track', artist: 'Artist', title: 'Song Name', createdAt: 0, ...overrides };
}

describe('matchesEntry', () => {
  it('requires the artist to match, ignoring case and extra whitespace', () => {
    expect(matchesEntry(station({ artist: '  ARTIST ' }), entry())).toBe(true);
    expect(matchesEntry(station({ artist: 'Someone Else' }), entry())).toBe(false);
  });

  it('matches artist alerts on any title', () => {
    expect(matchesEntry(station({ title: 'Whatever Is On' }), entry({ type: 'artist', title: undefined }))).toBe(true);
  });

  it('compares track titles with the trailing two-digit marker stripped', () => {
    expect(matchesEntry(station({ title: 'Song Name (93)' }), entry())).toBe(true);
    expect(matchesEntry(station({ title: 'Song Name (Live)' }), entry())).toBe(false);
  });

  it('does not match a track alert with a missing title against a real title', () => {
    expect(matchesEntry(station(), entry({ title: undefined }))).toBe(false);
  });

  it('does not throw when live station metadata is null', () => {
    const partial = { ...station(), artist: null, title: null } as unknown as StellarStation;
    expect(() => matchesEntry(partial, entry())).not.toThrow();
    expect(matchesEntry(partial, entry())).toBe(false);
  });
});
