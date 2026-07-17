import { describe, expect, it } from 'vitest';
import type { StellarChannelLogos } from '../types/stellarTunerLog';
import { pickChannelLogoUrl } from './channelLogo';

function logo(url: string) {
  return { url, width: 300, height: 300 };
}

const allLogos: StellarChannelLogos = {
  color_dark_square: logo('dark.png'),
  list_view_square: logo('list.png'),
  white_square: logo('white.png'),
};

describe('pickChannelLogoUrl', () => {
  it('prefers the opaque list-view tile in light mode', () => {
    expect(pickChannelLogoUrl(allLogos, 'light')).toBe('list.png');
  });

  it('prefers the transparent dark variant in dark mode', () => {
    expect(pickChannelLogoUrl(allLogos, 'dark')).toBe('dark.png');
  });

  it('falls back down the preference order when variants are missing', () => {
    expect(pickChannelLogoUrl({ white_square: logo('white.png') }, 'light')).toBe('white.png');
    expect(pickChannelLogoUrl({ white_square: logo('white.png') }, 'dark')).toBe('white.png');
    // Light mode still uses the dark variant when nothing else exists.
    expect(pickChannelLogoUrl({ color_dark_square: logo('dark.png') }, 'light')).toBe('dark.png');
  });

  it('skips variants with an empty url', () => {
    const logos: StellarChannelLogos = {
      list_view_square: logo(''),
      white_square: logo('white.png'),
    };
    expect(pickChannelLogoUrl(logos, 'light')).toBe('white.png');
  });

  it('returns undefined with no usable logos', () => {
    expect(pickChannelLogoUrl(undefined, 'light')).toBeUndefined();
    expect(pickChannelLogoUrl({}, 'dark')).toBeUndefined();
  });
});
