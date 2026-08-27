import { describe, expect, it } from 'vitest';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarStation } from '../types/stellarTunerLog';
import {
  MATCH_THRESHOLD,
  buildChannelMetadataMap,
  buildNowPlayingMap,
  findBestStationMatch,
  listUnmatchedChannels,
  nameSimilarity,
  normalizeChannelName,
  nowPlayingMapsEqual,
  parseChannelName,
  significantTokens,
} from './channelMatcher';

function channel(streamId: number, name: string): XtreamChannel {
  return { stream_id: streamId, name, stream_icon: '', num: streamId, category_id: '1' };
}

function station(overrides: Partial<StellarStation> = {}): StellarStation {
  return {
    id: 'station',
    name: 'Octane',
    channel_number: 37,
    artist: 'Artist',
    title: 'Title',
    album: 'Album',
    cut_type: 'Song',
    artwork_url: 'art.png',
    itunes_id: '',
    ...overrides,
  };
}

function stellarChannel(overrides: Partial<StellarChannel> = {}): StellarChannel {
  return {
    id: 'chan',
    name: 'Octane',
    marketing_name: 'Octane',
    channel_number: 37,
    categories: [],
    ...overrides,
  };
}

describe('normalizeChannelName', () => {
  it('strips the Radio: prefix, a leading The, and HD/Radio suffixes', () => {
    expect(normalizeChannelName('Radio: The Pulse HD')).toBe('pulse');
    expect(normalizeChannelName('Elvis Radio')).toBe('elvis');
    expect(normalizeChannelName('The Beatles Channel')).toBe('beatleschannel');
  });

  it('drops punctuation and whitespace so formatting differences do not matter', () => {
    expect(normalizeChannelName("70s on 7")).toBe('70son7');
    expect(normalizeChannelName('Hits 1')).toBe(normalizeChannelName('HITS-1'));
  });

  it('only strips Radio as a suffix or a "Radio:" prefix, not mid-name', () => {
    expect(normalizeChannelName('Radio Margaritaville')).toBe('radiomargaritaville');
  });
});

describe('nameSimilarity', () => {
  it('returns 1 for identical and 1 for two empty strings', () => {
    expect(nameSimilarity('octane', 'octane')).toBe(1);
    expect(nameSimilarity('', '')).toBe(1);
  });

  it('scales with edit distance over the longer length', () => {
    // one edit over length 6
    expect(nameSimilarity('octane', 'octant')).toBeCloseTo(1 - 1 / 6);
    expect(nameSimilarity('octane', 'zzzzzz')).toBe(0);
  });

  it('is symmetric', () => {
    expect(nameSimilarity('abc', 'abcd')).toBe(nameSimilarity('abcd', 'abc'));
  });
});

describe('findBestStationMatch', () => {
  const stations = [
    station({ id: 'octane', name: 'Octane' }),
    station({ id: 'pulse', name: 'The Pulse' }),
    station({ id: 'liquid', name: 'Liquid Metal' }),
  ];

  it('matches despite prefix/suffix and formatting noise', () => {
    expect(findBestStationMatch('Radio: The Pulse HD', stations)?.id).toBe('pulse');
    expect(findBestStationMatch('OCTANE', stations)?.id).toBe('octane');
  });

  it('returns null when the best score is below the threshold', () => {
    expect(findBestStationMatch('Willie’s Roadhouse', stations)).toBeNull();
  });

  it('picks the highest-scoring station, not the first acceptable one', () => {
    const close = [
      station({ id: 'near', name: 'Octanes' }),
      station({ id: 'exact', name: 'Octane' }),
    ];
    expect(findBestStationMatch('Octane', close)?.id).toBe('exact');
  });

  it('enforces the documented threshold at the boundary', () => {
    // "octanee" vs "octane": 1 edit / 7 chars ≈ 0.857 >= 0.85
    expect(nameSimilarity('octane', 'octanee')).toBeGreaterThanOrEqual(MATCH_THRESHOLD);
    expect(findBestStationMatch('Octanee', [station({ id: 'o', name: 'Octane' })])?.id).toBe('o');
  });
});

describe('buildNowPlayingMap', () => {
  it('matches channels to stations and populates the cache', () => {
    const cache = new Map<number, string>();
    const map = buildNowPlayingMap(
      [channel(1, 'Octane'), channel(2, 'Nowhere FM')],
      [station({ id: 's1', name: 'Octane' })],
      cache,
    );
    expect(map.get(1)?.id).toBe('s1');
    expect(map.has(2)).toBe(false);
    expect(cache.get(1)).toBe('s1');
    expect(cache.has(2)).toBe(false);
  });

  it('uses the cached station id without re-running the fuzzy match', () => {
    const cache = new Map<number, string>([[1, 's1']]);
    // Renamed beyond any fuzzy match - only the cached id can link these.
    const map = buildNowPlayingMap(
      [channel(1, 'Octane')],
      [station({ id: 's1', name: 'Completely Different Name', title: 'Fresh Song' })],
      cache,
    );
    expect(map.get(1)?.title).toBe('Fresh Song');
  });

  it('re-matches when the cached station drops out of the response', () => {
    const cache = new Map<number, string>([[1, 'gone']]);
    const map = buildNowPlayingMap(
      [channel(1, 'Octane')],
      [station({ id: 's2', name: 'Octane' })],
      cache,
    );
    expect(map.get(1)?.id).toBe('s2');
    expect(cache.get(1)).toBe('s2');
  });

  it('omits a channel whose cached station is gone and no longer matches anything', () => {
    const cache = new Map<number, string>([[1, 'gone']]);
    const map = buildNowPlayingMap(
      [channel(1, 'Octane')],
      [station({ id: 's3', name: 'Liquid Metal' })],
      cache,
    );
    expect(map.has(1)).toBe(false);
  });

  it('reuses the previous station object identity when rendered fields are unchanged', () => {
    const cache = new Map<number, string>([[1, 's1']]);
    const prevStation = station({ id: 's1', title: 'Same Song' });
    const prev = new Map([[1, prevStation]]);
    // A fresh response object with identical rendered fields (different object identity).
    const next = buildNowPlayingMap(
      [channel(1, 'Octane')],
      [station({ id: 's1', title: 'Same Song' })],
      cache,
      prev,
    );
    expect(next.get(1)).toBe(prevStation);
  });

  it('gives a new station object only to the channel whose track changed', () => {
    const cache = new Map<number, string>([[1, 's1'], [2, 's2']]);
    const s1Prev = station({ id: 's1', title: 'A' });
    const s2Prev = station({ id: 's2', title: 'B' });
    const prev = new Map([[1, s1Prev], [2, s2Prev]]);
    const next = buildNowPlayingMap(
      [channel(1, 'One'), channel(2, 'Two')],
      [station({ id: 's1', title: 'A' }), station({ id: 's2', title: 'B2' })],
      cache,
      prev,
    );
    // channel 1 unchanged -> reused identity; channel 2 changed -> new object
    expect(next.get(1)).toBe(s1Prev);
    expect(next.get(2)).not.toBe(s2Prev);
    expect(next.get(2)?.title).toBe('B2');
  });
});

describe('nowPlayingMapsEqual', () => {
  const base = () => new Map([[1, station({ id: 's1' })]]);

  it('treats maps with identical rendered fields as equal', () => {
    expect(nowPlayingMapsEqual(base(), base())).toBe(true);
  });

  it('detects a changed track on the same station', () => {
    const next = new Map([[1, station({ id: 's1', title: 'Other' })]]);
    expect(nowPlayingMapsEqual(base(), next)).toBe(false);
  });

  it('detects size changes and missing entries', () => {
    expect(nowPlayingMapsEqual(base(), new Map())).toBe(false);
    const next = new Map([[2, station({ id: 's1' })]]);
    expect(nowPlayingMapsEqual(base(), next)).toBe(false);
  });

  it('ignores fields that are not rendered', () => {
    const next = new Map([[1, station({ id: 's1', itunes_id: 'different' })]]);
    expect(nowPlayingMapsEqual(base(), next)).toBe(true);
  });
});

describe('buildChannelMetadataMap', () => {
  it('matches on marketing_name when present, falling back to name', () => {
    const byMarketing = stellarChannel({ id: 'a', name: 'internal-a', marketing_name: 'Octane' });
    const byName = stellarChannel({ id: 'b', name: 'The Pulse', marketing_name: '' });
    const map = buildChannelMetadataMap(
      [channel(1, 'Octane'), channel(2, 'The Pulse')],
      [byMarketing, byName],
    );
    expect(map.get(1)?.id).toBe('a');
    expect(map.get(2)?.id).toBe('b');
  });

  it('leaves unmatched channels out of the map', () => {
    const map = buildChannelMetadataMap([channel(1, 'Nowhere FM')], [stellarChannel()]);
    expect(map.size).toBe(0);
  });

  it('uses the cached id without re-running the fuzzy match', () => {
    const cache = new Map<number, string>([[1, 'cached']]);
    // Renamed beyond any fuzzy match - only the cached id can link these.
    const map = buildChannelMetadataMap(
      [channel(1, 'Octane')],
      [stellarChannel({ id: 'cached', name: 'Completely Different', marketing_name: 'Completely Different' })],
      cache,
    );
    expect(map.get(1)?.id).toBe('cached');
  });

  it('populates the cache on a fresh match', () => {
    const cache = new Map<number, string>();
    buildChannelMetadataMap([channel(1, 'Octane')], [stellarChannel({ id: 'a', marketing_name: 'Octane' })], cache);
    expect(cache.get(1)).toBe('a');
  });
});

describe('parseChannelName', () => {
  it('splits off a leading channel-number prefix', () => {
    expect(parseChannelName('43 Rock The Bells Radio')).toEqual({ number: 43, name: 'Rock The Bells Radio' });
    expect(parseChannelName('45 Shade 45')).toEqual({ number: 45, name: 'Shade 45' });
    expect(parseChannelName('56-The Highway')).toEqual({ number: 56, name: 'The Highway' });
    expect(parseChannelName('#52 BPM')).toEqual({ number: 52, name: 'BPM' });
    expect(parseChannelName('Ch. 37 Octane')).toEqual({ number: 37, name: 'Octane' });
  });

  it('drops a leading provider/locale tag', () => {
    expect(parseChannelName('US | 52 BPM')).toEqual({ number: 52, name: 'BPM' });
    expect(parseChannelName('SXM: Octane')).toEqual({ number: null, name: 'Octane' });
  });

  it('reads a trailing channel number too', () => {
    expect(parseChannelName('Octane - 37')).toEqual({ number: 37, name: 'Octane' });
    expect(parseChannelName('Octane (37)')).toEqual({ number: 37, name: 'Octane' });
  });

  it('leaves names whose digits are part of the name alone', () => {
    // The digits must be a whole token followed by a separator - in all of
    // these a letter follows them directly, so nothing is stripped.
    expect(parseChannelName('70s on 7')).toEqual({ number: null, name: '70s on 7' });
    expect(parseChannelName('40s Junction')).toEqual({ number: null, name: '40s Junction' });
    expect(parseChannelName('Pop2K')).toEqual({ number: null, name: 'Pop2K' });
    expect(parseChannelName('Flex2K')).toEqual({ number: null, name: 'Flex2K' });
  });

  it('never strips everything away', () => {
    expect(parseChannelName('3293')).toEqual({ number: null, name: '3293' });
    expect(parseChannelName('52 -')).toEqual({ number: null, name: '52 -' });
  });
});

describe('normalizeChannelName (provider noise)', () => {
  it('strips stacked quality and locale suffixes', () => {
    expect(normalizeChannelName('The Highway HD')).toBe('highway');
    expect(normalizeChannelName('The Highway (FHD)')).toBe('highway');
    expect(normalizeChannelName('Octane 1080p')).toBe('octane');
    expect(normalizeChannelName('Octane Radio HD [US]')).toBe('octane');
  });

  it('strips a leading provider tag only when a separator marks it as one', () => {
    expect(normalizeChannelName('US | Octane')).toBe('octane');
    expect(normalizeChannelName('SiriusXM Chill')).toBe('siriusxmchill');
  });

  it('treats & and "and" alike, and folds diacritics', () => {
    expect(normalizeChannelName('Heart & Soul')).toBe(normalizeChannelName('Heart and Soul'));
    expect(normalizeChannelName('Björk Radio')).toBe('bjork');
  });

  it('never strips a name down to nothing', () => {
    expect(normalizeChannelName('Radio')).toBe('radio');
  });
});

describe('significantTokens', () => {
  it('drops stopwords and is order-insensitive', () => {
    expect(significantTokens('The Groove')).toEqual(new Set(['groove']));
    expect(significantTokens('Radio Margaritaville')).toEqual(significantTokens('Margaritaville Radio'));
    expect(significantTokens('Heart & Soul')).toEqual(significantTokens('Heart and Soul'));
  });
});

describe('findBestStationMatch against provider-numbered channel names', () => {
  // A provider that prefixes every name with the SiriusXM channel number.
  // Under a single 0.85 edit-distance threshold these matched or failed purely
  // on how long the rest of the name was ("61 Willie's Roadhouse" scored 0.889
  // and matched; "52 BPM" scored 0.600 and did not), so this table is the
  // regression guard for the whole ladder.
  const lineup: [number, string][] = [
    [43, 'Rock The Bells Radio'],
    [44, 'Hip-Hop Nation'],
    [45, 'Shade 45'],
    [46, 'The Heat'],
    [47, 'Heart & Soul'],
    [48, 'The Flow'],
    [49, 'Flex2K'],
    [50, 'SiriusXM FLY'],
    [51, 'The Groove'],
    [52, 'BPM'],
    [53, "Diplo's Revolution"],
    [54, 'Studio 54 Radio'],
    [55, 'SiriusXM Chill'],
    [56, 'The Highway'],
    [57, 'Y2Kountry'],
    [58, 'Prime Country'],
    [59, 'No Shoes Radio'],
    [60, "Carrie's Country"],
    [61, "Willie's Roadhouse"],
    [62, 'Outlaw Country'],
  ];
  const stations = lineup.map(([channelNumber, name]) =>
    station({ id: `s${channelNumber}`, name, channel_number: channelNumber }),
  );

  it.each(lineup)('matches "%s %s" to the right station', (channelNumber, name) => {
    expect(findBestStationMatch(`${channelNumber} ${name}`, stations)?.id).toBe(`s${channelNumber}`);
  });

  it('also matches those names carrying quality/locale noise', () => {
    expect(findBestStationMatch('US | 52 BPM HD', stations)?.id).toBe('s52');
    expect(findBestStationMatch('56 The Highway (FHD)', stations)?.id).toBe('s56');
    expect(findBestStationMatch('47 Heart and Soul', stations)?.id).toBe('s47');
  });

  it('still tells neighbouring stations apart', () => {
    expect(findBestStationMatch('58 Prime Country', stations)?.id).toBe('s58');
    expect(findBestStationMatch('62 Outlaw Country', stations)?.id).toBe('s62');
    expect(findBestStationMatch('60 Carrie’s Country', stations)?.id).toBe('s60');
    expect(findBestStationMatch('99 Nowhere FM', stations)).toBeNull();
  });

  it('lets the name win over a prefix that is not a channel number', () => {
    // Some providers prefix a group/EPG number instead - the name still decides.
    expect(findBestStationMatch('22 - Shade 45', stations)?.id).toBe('s45');
  });

  it('rejects a channel-number hit that the name does not corroborate', () => {
    const numbered = [station({ id: 'unrelated', name: 'Liquid Metal', channel_number: 40 })];
    expect(findBestStationMatch('40 Watercolors', numbered)).toBeNull();
  });

  it('accepts a channel-number hit the name loosely corroborates', () => {
    const numbered = [
      station({ id: 'octane', name: 'Octane', channel_number: 37 }),
      station({ id: 'liquid', name: 'Liquid Metal', channel_number: 40 }),
    ];
    // Too mangled for the 0.85 fuzzy threshold, but the number agrees and the
    // name is still recognisably the same station.
    expect(findBestStationMatch('37 Octane Rock', numbered)?.id).toBe('octane');
  });
});

describe('findBestStationMatch word-order and stopword differences', () => {
  it('matches the same words in a different order', () => {
    const stations = [station({ id: 'marg', name: 'Radio Margaritaville' })];
    expect(findBestStationMatch('Margaritaville Radio', stations)?.id).toBe('marg');
  });

  it('does not match two stations that merely share a word', () => {
    const stations = [station({ id: 'prime', name: 'Prime Country' })];
    expect(findBestStationMatch('Outlaw Country', stations)).toBeNull();
  });
});

describe('buildChannelMetadataMap name sources', () => {
  it('matches a provider-numbered name to the Stellar channel', () => {
    const map = buildChannelMetadataMap(
      [channel(1, '43 Rock The Bells Radio')],
      [stellarChannel({ id: 'rtb', name: 'Rock The Bells Radio', marketing_name: 'Rock The Bells Radio', channel_number: 43 })],
    );
    expect(map.get(1)?.id).toBe('rtb');
  });

  it('falls back to the streaming name when the marketing name differs', () => {
    const map = buildChannelMetadataMap(
      [channel(1, '52 BPM')],
      [stellarChannel({ id: 'bpm', name: 'siriusxm-bpm', marketing_name: 'Beats Per Minute', streaming_name: 'BPM', channel_number: 52 })],
    );
    expect(map.get(1)?.id).toBe('bpm');
  });
});

describe('listUnmatchedChannels', () => {
  it('reports only unmatched channels, with the parsed name and number', () => {
    const metadata = new Map([[1, stellarChannel()]]);
    const unmatched = listUnmatchedChannels(
      [channel(1, '37 Octane'), channel(2, '52 BPM')],
      metadata,
    );
    expect(unmatched).toEqual([
      { streamId: 2, num: 2, rawName: '52 BPM', parsedNumber: 52, parsedName: 'BPM' },
    ]);
  });
});

describe('findBestStationMatch with decade-style and duplicated names', () => {
  const stations = [
    station({ id: 's5', name: '50s on 5', channel_number: 5 }),
    station({ id: 's7', name: '70s on 7', channel_number: 7 }),
    station({ id: 's8', name: '80s on 8', channel_number: 8 }),
    station({ id: 's10', name: 'Pop2K', channel_number: 10 }),
    station({ id: 's34', name: 'BPM', channel_number: 34 }),
    station({ id: 's52', name: 'BPM', channel_number: 52 }),
    station({ id: 's71', name: '40s Junction', channel_number: 71 }),
  ];

  it('keeps digits that belong to the name, prefix or not', () => {
    expect(findBestStationMatch('7 70s on 7 HD', stations)?.id).toBe('s7');
    expect(findBestStationMatch('8 80s on 8', stations)?.id).toBe('s8');
    expect(findBestStationMatch('10 Pop2K', stations)?.id).toBe('s10');
    expect(findBestStationMatch('71 40s Junction', stations)?.id).toBe('s71');
  });

  it('uses the channel number to break a tie between identically named stations', () => {
    expect(findBestStationMatch('34 BPM', stations)?.id).toBe('s34');
    expect(findBestStationMatch('52 BPM', stations)?.id).toBe('s52');
  });
});
