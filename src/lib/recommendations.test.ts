import { describe, expect, it } from 'vitest';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel } from '../types/stellarTunerLog';
import {
  CHANNELS_PER_ROW,
  buildRecommendationRows,
  getAllGenres,
  rankPersonalizedGenres,
  shuffleGenres,
} from './recommendations';

function channel(streamId: number): XtreamChannel {
  return { stream_id: streamId, name: `Channel ${streamId}`, stream_icon: '', num: streamId, category_id: '1' };
}

function meta(id: string, categories: Array<[name: string, isPrimary: boolean]>): StellarChannel {
  return {
    id,
    name: id,
    marketing_name: id,
    channel_number: 0,
    categories: categories.map(([name, is_primary], order) => ({ name, order, is_primary })),
  };
}

describe('rankPersonalizedGenres', () => {
  it('weights recent plays by recency position', () => {
    const metadata = new Map([
      [1, meta('a', [['Rock', true]])],
      [2, meta('b', [['Pop', true]])],
    ]);
    // Rock listened most recently (weight 1), Pop second (weight 1/2).
    expect(rankPersonalizedGenres(metadata, [1, 2], [])).toEqual(['Rock', 'Pop']);
    // Reversed order flips the ranking.
    expect(rankPersonalizedGenres(metadata, [2, 1], [])).toEqual(['Pop', 'Rock']);
  });

  it('counts secondary categories at half weight', () => {
    const metadata = new Map([[1, meta('a', [['Rock', true], ['Metal', false]])]]);
    const ranked = rankPersonalizedGenres(metadata, [1], []);
    expect(ranked).toEqual(['Rock', 'Metal']);
  });

  it('lets favorites outweigh recency', () => {
    const metadata = new Map([
      [1, meta('a', [['Rock', true]])],
      [2, meta('b', [['Jazz', true]])],
    ]);
    // Rock gets recency weight 1; Jazz gets the favorite bonus of 3.
    expect(rankPersonalizedGenres(metadata, [1], [2])).toEqual(['Jazz', 'Rock']);
  });

  it('breaks score ties alphabetically', () => {
    const metadata = new Map([[1, meta('a', [['Zeta', true], ['Alpha', true]])]]);
    expect(rankPersonalizedGenres(metadata, [1], [])).toEqual(['Alpha', 'Zeta']);
  });

  it('ignores plays with no metadata and returns empty with no history', () => {
    const metadata = new Map([[1, meta('a', [['Rock', true]])]]);
    expect(rankPersonalizedGenres(metadata, [999], [])).toEqual([]);
    expect(rankPersonalizedGenres(metadata, [], [])).toEqual([]);
  });
});

describe('getAllGenres', () => {
  it('collects unique category names across channels', () => {
    const metadata = new Map([
      [1, meta('a', [['Rock', true], ['Metal', false]])],
      [2, meta('b', [['Rock', true], ['Pop', true]])],
    ]);
    expect(getAllGenres(metadata).sort()).toEqual(['Metal', 'Pop', 'Rock']);
  });
});

describe('shuffleGenres', () => {
  it('shuffles deterministically for a fixed random source without mutating input', () => {
    const genres = ['a', 'b', 'c', 'd'];
    const shuffled = shuffleGenres(genres, () => 0);
    expect(shuffled).toEqual(['b', 'c', 'd', 'a']);
    expect(genres).toEqual(['a', 'b', 'c', 'd']);
  });

  it('keeps the same members', () => {
    const genres = ['a', 'b', 'c', 'd', 'e'];
    expect(shuffleGenres(genres).sort()).toEqual(genres);
  });
});

describe('buildRecommendationRows', () => {
  const channels = [channel(1), channel(2), channel(3)];
  const metadata = new Map([
    [1, meta('a', [['Rock', true]])],
    [2, meta('b', [['Rock', false], ['Pop', true]])],
    [3, meta('c', [['Jazz', true]])],
  ]);

  it('builds personalized rows first, then filler, without repeating genres', () => {
    const rows = buildRecommendationRows(['Rock'], ['Rock', 'Jazz'], channels, metadata, []);
    expect(rows.map((r) => r.genre)).toEqual(['Rock', 'Jazz']);
    expect(rows[0].personalized).toBe(true);
    expect(rows[0].channels.map((c) => c.stream_id)).toEqual([1, 2]);
    expect(rows[1].personalized).toBe(false);
    expect(rows[1].channels.map((c) => c.stream_id)).toEqual([3]);
  });

  it('excludes recently played channels and drops rows that end up empty', () => {
    const rows = buildRecommendationRows(['Jazz'], ['Rock'], channels, metadata, [3]);
    // Jazz's only channel was recently played, so no Jazz row at all.
    expect(rows.map((r) => r.genre)).toEqual(['Rock']);
    expect(rows[0].channels.map((c) => c.stream_id)).toEqual([1, 2]);
  });

  it('respects the row cap', () => {
    const rows = buildRecommendationRows(['Rock', 'Pop'], ['Jazz'], channels, metadata, [], 1);
    expect(rows).toHaveLength(1);
    expect(rows[0].genre).toBe('Rock');
  });

  it('caps each row at CHANNELS_PER_ROW channels', () => {
    const many = Array.from({ length: CHANNELS_PER_ROW + 3 }, (_, i) => channel(i + 1));
    const manyMeta = new Map(many.map((c) => [c.stream_id, meta(`m${c.stream_id}`, [['Rock', true]])]));
    const rows = buildRecommendationRows(['Rock'], [], many, manyMeta, []);
    expect(rows[0].channels).toHaveLength(CHANNELS_PER_ROW);
  });
});
