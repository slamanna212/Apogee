import { describe, expect, it } from 'vitest';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel } from '../types/stellarTunerLog';
import {
  channelDisplayName,
  channelDisplayNumber,
  cleanChannelName,
  isChannelUnmatched,
} from './channelDisplay';

function channel(name: string, num = 3293): XtreamChannel {
  return { stream_id: 1, name, stream_icon: '', num, category_id: '1' };
}

const metadata: StellarChannel = {
  id: 'rtb',
  name: 'Rock The Bells Radio',
  marketing_name: 'Rock The Bells Radio',
  channel_number: 43,
  categories: [],
};

describe('cleanChannelName', () => {
  it('drops the channel-number prefix and quality noise, keeping the name', () => {
    expect(cleanChannelName('43 Rock The Bells Radio')).toBe('Rock The Bells Radio');
    expect(cleanChannelName('56 The Highway HD')).toBe('The Highway');
    expect(cleanChannelName('US | 52 BPM (FHD)')).toBe('BPM');
  });

  it('leaves a name whose digits belong to it alone', () => {
    expect(cleanChannelName('70s on 7')).toBe('70s on 7');
    expect(cleanChannelName('45 Shade 45')).toBe('Shade 45');
  });

  it('never returns an empty name', () => {
    expect(cleanChannelName('HD')).toBe('HD');
  });
});

describe('channelDisplayName / channelDisplayNumber', () => {
  it('prefers StellarTunerLog metadata when the channel matched', () => {
    expect(channelDisplayName(channel('43 Rock The Bells Radio'), metadata)).toBe('Rock The Bells Radio');
    expect(channelDisplayNumber(channel('43 Rock The Bells Radio'), metadata)).toBe(43);
  });

  it('falls back to the cleaned name and the parsed number when it did not', () => {
    expect(channelDisplayName(channel('43 Rock The Bells Radio'))).toBe('Rock The Bells Radio');
    expect(channelDisplayNumber(channel('43 Rock The Bells Radio'))).toBe(43);
  });

  it('falls back to the Xtream lineup number when the name carries none', () => {
    expect(channelDisplayNumber(channel('Octane', 3287))).toBe(3287);
  });
});

describe('isChannelUnmatched', () => {
  it('only flags a channel once the metadata fetch has completed', () => {
    expect(isChannelUnmatched(undefined, 'loaded')).toBe(true);
    expect(isChannelUnmatched(undefined, 'loading')).toBe(false);
    expect(isChannelUnmatched(undefined, 'error')).toBe(false);
    expect(isChannelUnmatched(metadata, 'loaded')).toBe(false);
  });
});
