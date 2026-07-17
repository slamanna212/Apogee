import type { StellarStation } from '../types/stellarTunerLog';
import type { AlertEntry } from '../types/alerts';

export function normalizeText(value: unknown): string {
  return typeof value === 'string' ? value.trim().toLowerCase().replace(/\s+/g, ' ') : '';
}

/** Removes StellarTunerLog's trailing two-digit title marker without changing case. */
export function stripTrailingTitleNumber(value: unknown): string {
  return typeof value === 'string' ? value.trim().replace(/\s*\(\d{2}\)\s*$/, '') : '';
}

/**
 * StellarTunerLog's live title sometimes carries a trailing 2-digit
 * parenthetical suffix (e.g. "Song Name (01)") that isn't always present
 * when the same song is actually playing, so strip it before comparing.
 */
export function normalizeTitle(value: unknown): string {
  return normalizeText(stripTrailingTitleNumber(value));
}

export function matchesEntry(station: StellarStation, entry: AlertEntry): boolean {
  if (normalizeText(station.artist) !== normalizeText(entry.artist)) return false;
  if (entry.type === 'artist') return true;
  return normalizeTitle(station.title) === normalizeTitle(entry.title ?? '');
}
