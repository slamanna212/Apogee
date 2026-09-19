import { describe, expect, it } from 'vitest';
import { extractTint, hslToHex, MONOCHROME_TINT, rgbToHsl } from './artworkColors';

function pixels(colors: [number, number, number, number][], counts: number[]): Uint8ClampedArray {
  const out: number[] = [];
  colors.forEach((c, i) => {
    for (let n = 0; n < counts[i]; n++) out.push(...c);
  });
  return new Uint8ClampedArray(out);
}

function hueOf(hex: string): number {
  return rgbToHsl(parseInt(hex.slice(1, 3), 16), parseInt(hex.slice(3, 5), 16), parseInt(hex.slice(5, 7), 16))[0];
}

describe('hslToHex', () => {
  it('converts primary hues', () => {
    expect(hslToHex([0, 1, 0.5])).toBe('#ff0000');
    expect(hslToHex([120, 1, 0.5])).toBe('#00ff00');
    expect(hslToHex([240, 1, 0.5])).toBe('#0000ff');
  });
});

describe('extractTint', () => {
  it('returns null when every pixel is transparent', () => {
    expect(extractTint(pixels([[255, 0, 0, 0]], [16]))).toBeNull();
  });

  it('picks the dominant color first and a hue-distinct second stop', () => {
    const tint = extractTint(pixels([[220, 40, 40, 255], [40, 60, 220, 255]], [60, 20]));
    expect(tint).not.toBeNull();
    const [a, b] = tint!;
    expect(hueOf(a)).toBeLessThan(10);
    expect(hueOf(b)).toBeGreaterThan(200);
  });

  it('falls back to a 40° hue shift when the art is a single hue', () => {
    const [a, b] = extractTint(pixels([[220, 40, 40, 255]], [64]))!;
    const diff = Math.abs(hueOf(b) - hueOf(a));
    expect(Math.min(diff, 360 - diff)).toBeGreaterThan(35);
  });

  it('clamps very dark and very light colored art into the readable range', () => {
    for (const color of [[70, 8, 8, 255], [255, 225, 225, 255]] as [number, number, number, number][]) {
      const [a] = extractTint(pixels([color], [64]))!;
      const [, s, l] = rgbToHsl(parseInt(a.slice(1, 3), 16), parseInt(a.slice(3, 5), 16), parseInt(a.slice(5, 7), 16));
      expect(s).toBeGreaterThanOrEqual(0.44);
      expect(l).toBeGreaterThanOrEqual(0.51);
      expect(l).toBeLessThanOrEqual(0.73);
    }
  });

  it('gives black-and-white art the neutral pair instead of inventing a hue from JPEG noise', () => {
    // Near-grays with a faint green cast, plus pure black and white.
    const art = pixels([[120, 127, 120, 255], [30, 34, 30, 255], [0, 0, 0, 255], [250, 250, 250, 255]], [300, 300, 200, 200]);
    expect(extractTint(art)).toEqual(MONOCHROME_TINT);
  });

  it('still reads color from mostly-gray art with a colored subject', () => {
    const [a] = extractTint(pixels([[128, 128, 128, 255], [220, 40, 40, 255]], [900, 124]))!;
    expect(a).not.toBe(MONOCHROME_TINT[0]);
    expect(rgbToHsl(parseInt(a.slice(1, 3), 16), parseInt(a.slice(3, 5), 16), parseInt(a.slice(5, 7), 16))[0]).toBeLessThan(10);
  });
});
