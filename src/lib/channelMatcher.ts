import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarStation } from '../types/stellarTunerLog';

export const MATCH_THRESHOLD = 0.85;

/**
 * How much the names must still agree before a channel-number match is
 * accepted. A bare number match is never enough on its own: providers that
 * prefix a group or EPG number rather than the SiriusXM channel number would
 * otherwise bind channels to whichever station happens to sit on that number.
 */
export const NUMBER_CORROBORATION_THRESHOLD = 0.5;

/** Leading provider/locale tag, but only when a separator makes it a tag ("US | Octane"). */
const SOURCE_TAG_PREFIX = /^(?:us|usa|uk|ca|sxm|siriusxm|sirius)\s*[|:\-–—]\s*/i;
/** Stream-quality/locale noise providers append ("Octane HD", "Octane (FHD)"). */
const QUALITY_SUFFIX = /[\s._-]+(?:hd|fhd|uhd|sd|4k|1080p?|720p?|hevc|h265)$/i;
const BRACKETED_SUFFIX = /\s*[([{][^)\]}]*[)\]}]$/;
const RADIO_SUFFIX = /[\s._-]+radio$/i;

/**
 * Peels off the quality/locale noise providers stack onto a name
 * ("The Highway HD (US)"), leaving the name itself untouched - `stripRadio`
 * additionally drops a trailing "Radio", which helps when matching but would
 * mangle the display name of a station actually called "... Radio".
 */
export function stripTrailingNoise(name: string, stripRadio = false): string {
  let value = name.trim();
  // Loop: these stack, and a station whose whole name is noise ("Radio") must
  // keep it rather than be stripped down to nothing.
  for (;;) {
    let stripped = value.replace(BRACKETED_SUFFIX, '').replace(QUALITY_SUFFIX, '');
    if (stripRadio) stripped = stripped.replace(RADIO_SUFFIX, '');
    stripped = stripped.trim();
    if (stripped === value || stripped.length === 0) break;
    value = stripped;
  }
  return value;
}

/** NFKD-folds and drops combining marks so "Björk FM" and "Bjork FM" normalize alike. */
function foldDiacritics(value: string): string {
  return value.normalize('NFKD').replace(/[̀-ͯ]/g, '');
}

export function normalizeChannelName(name: string): string {
  let normalized = foldDiacritics(name).trim().toLowerCase();
  normalized = normalized.replace(/&/g, ' and ');
  normalized = normalized.replace(SOURCE_TAG_PREFIX, '');
  normalized = normalized.replace(/^radio:\s*/, '');
  normalized = normalized.replace(/^the\s+/, '');
  normalized = stripTrailingNoise(normalized, true);
  normalized = normalized.replace(/[^a-z0-9]+/g, '');
  return normalized;
}

/**
 * A leading channel-number token: "43 Rock The Bells", "56-The Highway",
 * "#43 Octane", "Ch. 43 Octane".
 *
 * The digits must be a whole token followed by a separator or whitespace,
 * which is what keeps genuine names intact - "70s on 7", "Pop2K", "Flex2K"
 * and "40s Junction" all have a letter directly after the digits, so nothing
 * is stripped from them.
 */
const LEADING_NUMBER = /^(?:#|channel|ch\.?)?\s*(\d{1,4})\s*(?:[-–—:.|)\]]+\s*|\s+)(?=\S)/i;
/** The same idea at the end: "Octane - 43", "Octane (43)". */
const TRAILING_NUMBER = /\s*[-–—|([]\s*(?:#|ch\.?)?\s*(\d{1,4})\s*[)\]]?$/i;

export interface ParsedChannelName {
  /** The SiriusXM channel number the provider baked into the name, if any. */
  number: number | null;
  /** The name with that number (and any leading provider tag) removed. */
  name: string;
}

export function parseChannelName(raw: string): ParsedChannelName {
  const trimmed = raw.trim();
  const untagged = trimmed.replace(SOURCE_TAG_PREFIX, '').trim();

  const leading = untagged.match(LEADING_NUMBER);
  if (leading) {
    const rest = untagged.slice(leading[0].length).trim();
    if (/[a-z]/i.test(rest)) {
      return { number: toChannelNumber(leading[1]), name: rest };
    }
  }

  const trailing = untagged.match(TRAILING_NUMBER);
  if (trailing) {
    const rest = untagged.slice(0, untagged.length - trailing[0].length).trim();
    if (/[a-z]/i.test(rest)) {
      return { number: toChannelNumber(trailing[1]), name: rest };
    }
  }

  return { number: null, name: untagged.length > 0 ? untagged : trimmed };
}

function toChannelNumber(digits: string): number | null {
  const value = Number.parseInt(digits, 10);
  return Number.isFinite(value) && value > 0 ? value : null;
}

/** Words that carry no identity of their own when comparing station names. */
const TOKEN_STOPWORDS = new Set([
  'the', 'a', 'an', 'and', 'radio', 'hd', 'fhd', 'uhd', 'sd', '4k', '1080p', '720p', 'hevc', 'h265',
]);

/**
 * The order-insensitive word set of a name, minus stopwords - so
 * "Radio Margaritaville" and "Margaritaville Radio" compare as the same
 * station even though their edit distance is large.
 */
export function significantTokens(name: string): Set<string> {
  const tokens = foldDiacritics(name)
    .toLowerCase()
    .replace(/&/g, ' and ')
    .split(/[^a-z0-9]+/)
    .filter((token) => token.length > 0 && !TOKEN_STOPWORDS.has(token));
  return new Set(tokens);
}

function tokenSetsEqual(a: Set<string>, b: Set<string>): boolean {
  if (a.size === 0 || a.size !== b.size) return false;
  for (const token of a) {
    if (!b.has(token)) return false;
  }
  return true;
}

export interface NameVariant {
  normalized: string;
  tokens: Set<string>;
}

function toVariant(name: string): NameVariant {
  return { normalized: normalizeChannelName(name), tokens: significantTokens(name) };
}

/**
 * The forms of a name worth comparing: as written, and with any provider
 * channel-number prefix removed. Matching takes the best score across every
 * variant pair, so an added variant can only ever create a match - never
 * take away one the raw name already made.
 */
export function nameVariants(...names: (string | undefined | null)[]): NameVariant[] {
  const variants: NameVariant[] = [];
  const seen = new Set<string>();
  for (const name of names) {
    if (!name) continue;
    for (const form of [name, parseChannelName(name).name]) {
      const variant = toVariant(form);
      if (variant.normalized.length === 0 || seen.has(variant.normalized)) continue;
      seen.add(variant.normalized);
      variants.push(variant);
    }
  }
  return variants;
}

// Rolling two-row Levenshtein: O(min·max) time, O(min(len)) space, no per-call
// 2D matrix allocation (this is the innermost op of every fuzzy match below).
function levenshtein(a: string, b: string): number {
  if (a.length === 0) return b.length;
  if (b.length === 0) return a.length;
  let prev = new Array<number>(b.length + 1);
  let curr = new Array<number>(b.length + 1);
  for (let j = 0; j <= b.length; j++) prev[j] = j;
  for (let i = 1; i <= a.length; i++) {
    curr[0] = i;
    const ac = a[i - 1];
    for (let j = 1; j <= b.length; j++) {
      curr[j] =
        ac === b[j - 1]
          ? prev[j - 1]
          : 1 + Math.min(prev[j - 1], prev[j], curr[j - 1]);
    }
    const swap = prev;
    prev = curr;
    curr = swap;
  }
  return prev[b.length];
}

export function nameSimilarity(a: string, b: string): number {
  const maxLen = Math.max(a.length, b.length);
  if (maxLen === 0) return 1;
  return 1 - levenshtein(a, b) / maxLen;
}

/**
 * Given only the two lengths, the highest similarity `nameSimilarity` could
 * return is `min(len)/max(len)` (achieved when the shorter string is a pure
 * subsequence of the longer). Used to skip the O(n·m) Levenshtein entirely for
 * candidates that can't beat the current best / clear the threshold.
 */
function lengthUpperBound(aLen: number, bLen: number): number {
  const maxLen = Math.max(aLen, bLen);
  if (maxLen === 0) return 1;
  return Math.min(aLen, bLen) / maxLen;
}

/**
 * A candidate list with its per-item name variants, token sets and channel
 * numbers computed once, so callers matching many channels against the same
 * list don't redo that work per channel.
 */
export interface PreparedCandidates<T> {
  items: T[];
  variants: NameVariant[][];
  numbers: (number | undefined)[];
}

export function prepareCandidates<T>(
  items: T[],
  namesOf: (item: T) => (string | undefined | null)[],
  numberOf?: (item: T) => number | undefined,
): PreparedCandidates<T> {
  return {
    items,
    variants: items.map((item) => nameVariants(...namesOf(item))),
    numbers: items.map((item) => (numberOf ? numberOf(item) : undefined)),
  };
}

// Acceptance tiers, best first. A higher tier always wins; within a tier the
// higher name similarity wins, then a channel-number agreement, then order.
const TIER_EXACT = 4;
const TIER_FUZZY = 3;
const TIER_TOKENS = 2;
const TIER_NUMBER = 1;

/**
 * Picks the candidate that best identifies the same station as `xtreamName`.
 *
 * Xtream providers rename channels freely - number prefixes, quality suffixes,
 * reordered words - so a single edit-distance threshold over the whole string
 * is length-sensitive in a way that makes short names ("52 BPM") fail while
 * long ones ("61 Willie's Roadhouse") squeak through. Instead each candidate is
 * judged on the best of four signals, in confidence order:
 *
 *   1. an exact normalized-name hit on any variant pair,
 *   2. edit-distance similarity at or above `MATCH_THRESHOLD`,
 *   3. an identical set of significant words (order- and stopword-insensitive),
 *   4. a channel-number hit corroborated by loose name agreement.
 */
export function findBestMatch<T>(xtreamName: string, candidates: PreparedCandidates<T>): T | null {
  const targets = nameVariants(xtreamName);
  if (targets.length === 0) return null;
  const targetNumber = parseChannelName(xtreamName).number;

  let best: T | null = null;
  let bestTier = 0;
  let bestScore = 0;
  let bestNumberMatch = false;

  for (let i = 0; i < candidates.items.length; i++) {
    const number = candidates.numbers[i];
    const numberMatch = targetNumber !== null && number !== undefined && number === targetNumber;
    // A corroborated number match accepts at a lower bar, so the length prune
    // has to stay below that bar too or tier 4 candidates never get scored.
    const scoreFloor = numberMatch ? NUMBER_CORROBORATION_THRESHOLD : MATCH_THRESHOLD;

    let score = 0;
    let exact = false;
    let tokensEqual = false;

    for (const target of targets) {
      for (const candidate of candidates.variants[i]) {
        if (target.normalized === candidate.normalized) {
          exact = true;
          score = 1;
          break;
        }
        if (!tokensEqual && tokenSetsEqual(target.tokens, candidate.tokens)) {
          tokensEqual = true;
        }
        const bound = lengthUpperBound(target.normalized.length, candidate.normalized.length);
        if (bound <= score || bound < scoreFloor) continue;
        const maxLen = Math.max(target.normalized.length, candidate.normalized.length);
        const pairScore = 1 - levenshtein(target.normalized, candidate.normalized) / maxLen;
        if (pairScore > score) score = pairScore;
      }
      if (exact) break;
    }

    let tier = 0;
    if (exact) tier = TIER_EXACT;
    else if (score >= MATCH_THRESHOLD) tier = TIER_FUZZY;
    else if (tokensEqual) tier = TIER_TOKENS;
    else if (numberMatch && score >= NUMBER_CORROBORATION_THRESHOLD) tier = TIER_NUMBER;
    if (tier === 0) continue;

    const better =
      tier > bestTier ||
      (tier === bestTier &&
        (score > bestScore || (score === bestScore && numberMatch && !bestNumberMatch)));
    if (better) {
      best = candidates.items[i];
      bestTier = tier;
      bestScore = score;
      bestNumberMatch = numberMatch;
    }
  }

  return best;
}

export function prepareStationCandidates(stations: StellarStation[]): PreparedCandidates<StellarStation> {
  return prepareCandidates(stations, (station) => [station.name], (station) => station.channel_number);
}

export function findBestStationMatch(
  xtreamName: string,
  stations: StellarStation[],
  prepared?: PreparedCandidates<StellarStation>,
): StellarStation | null {
  return findBestMatch(xtreamName, prepared ?? prepareStationCandidates(stations));
}

/** The now-playing fields actually rendered by the channel cards / transport bar. */
function stationRenderEqual(a: StellarStation, b: StellarStation): boolean {
  return (
    a.id === b.id &&
    a.title === b.title &&
    a.artist === b.artist &&
    a.album === b.album &&
    a.cut_type === b.cut_type &&
    a.artwork_url === b.artwork_url
  );
}

/**
 * Builds the channel -> live-station now-playing map for one poll tick.
 *
 * The channel/station name match is stable (a channel doesn't change which
 * station it corresponds to between polls) while only the song/artist inside
 * a station changes - so `stationIdCache` remembers each channel's matched
 * `StellarStation.id` and this only re-runs the fuzzy match for channels that
 * aren't cached yet (first tick, or a channel whose station dropped out of the
 * current response), instead of re-matching every channel against every station
 * on every tick.
 *
 * When `prev` is supplied, each entry whose rendered fields are unchanged reuses
 * the previous `StellarStation` object identity, so a song change on one station
 * only gives a new object (and thus a new memo prop) to that one channel's card
 * rather than invalidating `React.memo` on every card.
 */
export function buildNowPlayingMap(
  channels: XtreamChannel[],
  stations: StellarStation[],
  stationIdCache: Map<number, string>,
  prev?: Map<number, StellarStation>,
): Map<number, StellarStation> {
  const stationsById = new Map(stations.map((station) => [station.id, station]));
  // Prepared once for this tick, and only if some channel actually needs a
  // fresh match (reused across every uncached channel below).
  let prepared: PreparedCandidates<StellarStation> | undefined;
  const map = new Map<number, StellarStation>();
  for (const channel of channels) {
    const cachedId = stationIdCache.get(channel.stream_id);
    let station = cachedId ? stationsById.get(cachedId) : undefined;
    if (!station) {
      if (!prepared) prepared = prepareStationCandidates(stations);
      const match = findBestMatch(channel.name, prepared);
      if (!match) continue;
      station = match;
      stationIdCache.set(channel.stream_id, match.id);
    }
    const previous = prev?.get(channel.stream_id);
    map.set(channel.stream_id, previous && stationRenderEqual(previous, station) ? previous : station);
  }
  return map;
}

/**
 * Shallow content comparison for two now-playing maps (by the fields
 * actually rendered), so a poll tick that returned identical data can keep
 * the previous `Map` reference instead of forcing a re-render everywhere
 * `nowPlaying` is subscribed to.
 */
export function nowPlayingMapsEqual(
  a: Map<number, StellarStation>,
  b: Map<number, StellarStation>,
): boolean {
  if (a.size !== b.size) return false;
  for (const [streamId, station] of b) {
    const prev = a.get(streamId);
    if (!prev || !stationRenderEqual(prev, station)) {
      return false;
    }
  }
  return true;
}

/**
 * Builds the channel -> Stellar-channel metadata map (logos/categories/etc).
 *
 * Like `buildNowPlayingMap`, the channel/Stellar-channel match is stable, so
 * `metadataIdCache` remembers each channel's matched `StellarChannel.id` and
 * only uncached channels run the fuzzy match. Every name StellarTunerLog knows
 * a channel by is offered as a candidate, since providers name channels after
 * any of them.
 */
export function buildChannelMetadataMap(
  channels: XtreamChannel[],
  stellarChannels: StellarChannel[],
  metadataIdCache?: Map<number, string>,
): Map<number, StellarChannel> {
  const byId = new Map(stellarChannels.map((c) => [c.id, c]));
  let prepared: PreparedCandidates<StellarChannel> | undefined;
  const map = new Map<number, StellarChannel>();
  for (const channel of channels) {
    const cachedId = metadataIdCache?.get(channel.stream_id);
    let match = cachedId ? byId.get(cachedId) : undefined;
    if (!match) {
      if (!prepared) {
        prepared = prepareCandidates(
          stellarChannels,
          (c) => [c.marketing_name, c.name, c.streaming_name],
          (c) => c.channel_number,
        );
      }
      const found = findBestMatch(channel.name, prepared);
      if (!found) continue;
      match = found;
      metadataIdCache?.set(channel.stream_id, found.id);
    }
    map.set(channel.stream_id, match);
  }
  return map;
}

export interface UnmatchedChannel {
  streamId: number;
  num: number;
  rawName: string;
  parsedNumber: number | null;
  parsedName: string;
}

/**
 * The channels that found no StellarTunerLog metadata, for the Settings
 * diagnostics table - the raw name next to what the matcher made of it is
 * usually enough to see how a provider names things.
 */
export function listUnmatchedChannels(
  channels: XtreamChannel[],
  channelMetadata: Map<number, StellarChannel>,
): UnmatchedChannel[] {
  const unmatched: UnmatchedChannel[] = [];
  for (const channel of channels) {
    if (channelMetadata.has(channel.stream_id)) continue;
    const parsed = parseChannelName(channel.name);
    unmatched.push({
      streamId: channel.stream_id,
      num: channel.num,
      rawName: channel.name,
      parsedNumber: parsed.number,
      parsedName: parsed.name,
    });
  }
  return unmatched;
}
