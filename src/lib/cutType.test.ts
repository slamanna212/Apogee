import { describe, expect, it } from 'vitest';
import { getCutTypeBadge } from './cutType';

describe('getCutTypeBadge', () => {
  it.each([
    ['Song', 'Song'],
    ['sports', 'Sports'],
    ['TALK', 'Talk'],
    ['PGM_Segment', 'Program'],
    ['Exp', 'Explicit'],
    ['Spot', 'Spot'],
    ['Promo', 'Promo'],
  ])('maps %s to the %s badge', (raw, label) => {
    expect(getCutTypeBadge(raw)?.label).toBe(label);
  });

  it('maps the known "PGM_Segement" typo variant to the same Program badge', () => {
    expect(getCutTypeBadge('PGM_Segement')).toBe(getCutTypeBadge('PGM_Segment'));
  });

  it('normalizes case and surrounding whitespace', () => {
    expect(getCutTypeBadge('  song  ')?.label).toBe('Song');
  });

  it('passes through link/perm/fill with their original casing as a dim badge', () => {
    const badge = getCutTypeBadge(' Perm ');
    expect(badge?.label).toBe('Perm');
    expect(badge?.color).toBe('var(--app-dim)');
    expect(getCutTypeBadge('link')?.label).toBe('link');
  });

  it('returns null for unknown, empty, and missing values', () => {
    expect(getCutTypeBadge('mystery')).toBeNull();
    expect(getCutTypeBadge('')).toBeNull();
    expect(getCutTypeBadge('   ')).toBeNull();
    expect(getCutTypeBadge(null)).toBeNull();
    expect(getCutTypeBadge(undefined)).toBeNull();
  });
});
