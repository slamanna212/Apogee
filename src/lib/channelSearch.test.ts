import { describe, expect, it } from 'vitest';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarStation } from '../types/stellarTunerLog';
import { channelMatchesSearch } from './channelSearch';

const channel: XtreamChannel = {
  stream_id: 37,
  name: 'Raw Octane Name',
  stream_icon: '',
  num: 37,
  category_id: '1',
};

const metadata = { marketing_name: 'Octane' } as StellarChannel;
const station = { title: 'Song Title', artist: 'Some Artist' } as StellarStation;

describe('channelMatchesSearch', () => {
  it('matches the display name, current title, and current artist', () => {
    expect(channelMatchesSearch(channel, metadata, station, 'oct')).toBe(true);
    expect(channelMatchesSearch(channel, metadata, station, 'song')).toBe(true);
    expect(channelMatchesSearch(channel, metadata, station, 'artist')).toBe(true);
  });

  it('returns false for a query with no results', () => {
    expect(channelMatchesSearch(channel, metadata, station, 'not present')).toBe(false);
  });

  it('treats null or missing remote track fields as empty strings', () => {
    const partialStation = { ...station, title: null, artist: undefined } as unknown as StellarStation;
    expect(() => channelMatchesSearch(channel, metadata, partialStation, 'not present')).not.toThrow();
    expect(channelMatchesSearch(channel, metadata, partialStation, 'not present')).toBe(false);
  });
});
