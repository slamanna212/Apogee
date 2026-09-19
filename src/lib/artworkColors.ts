import { invoke } from '@tauri-apps/api/core';

/** Two-stop tint for the transport rail: play button fill, its glow, the surface wash and
 *  the visualizer bars all read from this one pair (see TransportBar). */
export type TintPair = [string, string];

/** The existing `--app-accent` / `--app-accent2` pair in each scheme (src/theme.ts). */
export const STATIC_TINT: Record<'dark' | 'light', TintPair> = {
  dark: ['#8b6bff', '#45e0d8'],
  light: ['#6c47ff', '#20b8c9'],
};

/** For black-and-white art: light grays with a hint of the app's cool cast, kept inside the
 *  same 52-72% lightness band as art-derived pairs so the dark play glyph stays readable. */
export const MONOCHROME_TINT: TintPair = ['#b2b1bc', '#8c8a99'];

const SAMPLE_SIZE = 32;
/** A pixel counts as colored when its strongest and weakest RGB channels differ by at least
 *  this fraction of full scale. JPEG noise in grayscale art stays well below it. HSL
 *  saturation is no use here: it reads high for near-black pixels whatever their hue. */
const MIN_CHROMA = 0.1;
/** Below this share of colored pixels, the art is treated as black and white. */
const MIN_COLORED_SHARE = 0.08;
const MIN_HUE_DISTANCE = 35;
const SECOND_STOP_CANDIDATES = 11;

type Hsl = [number, number, number];

export function rgbToHsl(r: number, g: number, b: number): Hsl {
  r /= 255;
  g /= 255;
  b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const d = max - min;
  if (!d) return [0, 0, l];
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h: number;
  if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  return [h * 60, s, l];
}

export function hslToHex([h, s, l]: Hsl): string {
  h = ((h % 360) + 360) % 360;
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  const segments: [number, number, number][] = [[c, x, 0], [x, c, 0], [0, c, x], [0, x, c], [x, 0, c], [c, 0, x]];
  const seg = segments[Math.floor(h / 60) % 6];
  return '#' + seg.map((v) => Math.round((v + m) * 255).toString(16).padStart(2, '0')).join('');
}

/** Clamping saturation and lightness keeps the dark `--app-bg` play glyph above 4.5:1 on
 *  the button in either color scheme, whatever the artwork looks like. */
function normalize([h, s, l]: Hsl): string {
  return hslToHex([h, Math.min(0.9, Math.max(0.45, s)), Math.min(0.72, Math.max(0.52, l))]);
}

function hueDistance(a: number, b: number): number {
  const d = Math.abs(a - b);
  return Math.min(d, 360 - d);
}

/**
 * Picks two dominant, distinct colors from RGBA pixel data. Pixels are bucketed at 5 bits
 * per channel and each colored bucket is scored by `count * (0.3 + saturation)`, so a
 * large muted background doesn't automatically beat a smaller vivid subject.
 *
 * Black-and-white art gets the neutral pair instead. Without that check, the saturation
 * floor in `normalize` would blow the faint color noise in a gray cover up into a vivid,
 * made-up hue. Returns null when there is nothing opaque to sample.
 */
export function extractTint(data: ArrayLike<number>): TintPair | null {
  const buckets = new Map<number, { n: number; r: number; g: number; b: number }>();
  let opaque = 0;
  let colored = 0;
  for (let i = 0; i + 3 < data.length; i += 4) {
    if (data[i + 3] < 128) continue;
    const r = data[i];
    const g = data[i + 1];
    const b = data[i + 2];
    opaque++;
    if ((Math.max(r, g, b) - Math.min(r, g, b)) / 255 < MIN_CHROMA) continue;
    colored++;
    const key = ((r >> 3) << 10) | ((g >> 3) << 5) | (b >> 3);
    const bucket = buckets.get(key);
    if (bucket) {
      bucket.n++;
      bucket.r += r;
      bucket.g += g;
      bucket.b += b;
    } else {
      buckets.set(key, { n: 1, r, g, b });
    }
  }
  if (opaque === 0) return null;
  if (colored / opaque < MIN_COLORED_SHARE) return MONOCHROME_TINT;

  const scored = [...buckets.values()]
    .map((e) => {
      const hsl = rgbToHsl(e.r / e.n, e.g / e.n, e.b / e.n);
      return { hsl, weight: e.n * (0.3 + hsl[1]) };
    })
    .sort((a, b) => b.weight - a.weight);

  const a = scored[0].hsl;
  const b = scored
    .slice(1, 1 + SECOND_STOP_CANDIDATES)
    .find((s) => hueDistance(s.hsl[0], a[0]) > MIN_HUE_DISTANCE)?.hsl
    ?? ([(a[0] + 40) % 360, a[1], a[2]] as Hsl);
  return [normalize(a), normalize(b)];
}

/** Remote artwork is fetched by Rust (see src-tauri/src/artwork.rs): a canvas can only read
 *  back pixels from a cross-origin image when its host sends CORS headers. Data URLs (the
 *  Settings preview's sample art) are decoded through an <img>, because the CSP's img-src
 *  allows `data:` while its connect-src (which governs fetch) does not. */
async function loadBitmap(url: string): Promise<ImageBitmap> {
  if (url.startsWith('data:')) {
    const img = new Image();
    img.src = url;
    await img.decode();
    return createImageBitmap(img);
  }
  const bytes = await invoke<ArrayBuffer>('artwork_fetch', { url });
  return createImageBitmap(new Blob([bytes]));
}

async function computeTint(url: string): Promise<TintPair | null> {
  const bitmap = await loadBitmap(url);
  try {
    const canvas = document.createElement('canvas');
    canvas.width = SAMPLE_SIZE;
    canvas.height = SAMPLE_SIZE;
    const ctx = canvas.getContext('2d', { willReadFrequently: true });
    if (!ctx) return null;
    ctx.drawImage(bitmap, 0, 0, SAMPLE_SIZE, SAMPLE_SIZE);
    return extractTint(ctx.getImageData(0, 0, SAMPLE_SIZE, SAMPLE_SIZE).data);
  } finally {
    bitmap.close();
  }
}

const MAX_CACHE_ENTRIES = 64;
/** Keyed by artwork URL. A null entry records a failure so a broken URL isn't refetched on
 *  every render; callers fall back to the static pair for it. */
const cache = new Map<string, Promise<TintPair | null>>();

export function artworkTint(url: string): Promise<TintPair | null> {
  const cached = cache.get(url);
  if (cached) return cached;
  const pending = computeTint(url).catch(() => null);
  cache.set(url, pending);
  if (cache.size > MAX_CACHE_ENTRIES) {
    const oldest = cache.keys().next().value;
    if (oldest !== undefined) cache.delete(oldest);
  }
  return pending;
}
