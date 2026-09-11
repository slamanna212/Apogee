import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel } from '../types/stellarTunerLog';

export const RECENCY_WINDOW = 20;
export const SECONDARY_CATEGORY_WEIGHT = 0.5;
export const FAVORITE_BONUS = 3;
export const MAX_ROWS = 6;
export const CHANNELS_PER_ROW = 8;

export const HOME_PAGE_SIZES = {
  small: { maxRows: 3, channelsPerRow: 6, recentCount: 6 },
  standard: { maxRows: MAX_ROWS, channelsPerRow: CHANNELS_PER_ROW, recentCount: 10 },
  expanded: { maxRows: 10, channelsPerRow: 12, recentCount: 15 },
} as const;

export interface RecommendationRow {
  genre: string;
  channels: XtreamChannel[];
  personalized: boolean;
}

export function rankPersonalizedGenres(
  channelMetadata: Map<number, StellarChannel>,
  recentlyPlayed: number[],
  favorites: number[],
): string[] {
  const scores = new Map<string, number>();
  const add = (name: string, weight: number) => {
    scores.set(name, (scores.get(name) ?? 0) + weight);
  };

  recentlyPlayed.slice(0, RECENCY_WINDOW).forEach((streamId, i) => {
    const meta = channelMetadata.get(streamId);
    if (!meta) return;
    const w = 1 / (i + 1);
    for (const cat of meta.categories) {
      add(cat.name, cat.is_primary ? w : w * SECONDARY_CATEGORY_WEIGHT);
    }
  });

  for (const streamId of favorites) {
    const meta = channelMetadata.get(streamId);
    if (!meta) continue;
    for (const cat of meta.categories) {
      add(cat.name, cat.is_primary ? FAVORITE_BONUS : FAVORITE_BONUS * SECONDARY_CATEGORY_WEIGHT);
    }
  }

  return [...scores.entries()]
    .filter(([, score]) => score > 0)
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([name]) => name);
}

export function getAllGenres(channelMetadata: Map<number, StellarChannel>): string[] {
  const set = new Set<string>();
  for (const meta of channelMetadata.values()) {
    for (const cat of meta.categories) set.add(cat.name);
  }
  return [...set];
}

export function shuffleGenres(genres: string[], random: () => number = Math.random): string[] {
  const arr = [...genres];
  for (let i = arr.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1));
    [arr[i], arr[j]] = [arr[j], arr[i]];
  }
  return arr;
}

export function buildRecommendationRows(
  personalizedGenres: string[],
  fillerGenres: string[],
  channels: XtreamChannel[],
  channelMetadata: Map<number, StellarChannel>,
  recentlyPlayed: number[],
  maxRows: number = MAX_ROWS,
  channelsPerRow: number = CHANNELS_PER_ROW,
): RecommendationRow[] {
  const recentIds = new Set(recentlyPlayed);
  const used = new Set<string>();
  const rows: RecommendationRow[] = [];

  const channelsFor = (genre: string) =>
    channels.filter(
      (c) =>
        !recentIds.has(c.stream_id) &&
        channelMetadata.get(c.stream_id)?.categories.some((cat) => cat.name === genre),
    );

  for (const genre of personalizedGenres) {
    if (rows.length >= maxRows) break;
    used.add(genre);
    const matched = channelsFor(genre);
    if (matched.length > 0) {
      rows.push({ genre, channels: matched.slice(0, channelsPerRow), personalized: true });
    }
  }

  for (const genre of fillerGenres) {
    if (rows.length >= maxRows) break;
    if (used.has(genre)) continue;
    used.add(genre);
    const matched = channelsFor(genre);
    if (matched.length > 0) {
      rows.push({ genre, channels: matched.slice(0, channelsPerRow), personalized: false });
    }
  }

  return rows;
}
