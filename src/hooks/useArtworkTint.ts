import { useEffect, useState } from 'react';
import { useComputedColorScheme } from '@mantine/core';
import { artworkTint, STATIC_TINT, type TintPair } from '../lib/artworkColors';
import type { RailColorSource } from '../stores/settingsStore';

/**
 * The rail's two-stop tint. In 'artwork' mode it follows the current track art, keeping
 * the previous pair while the next one is being sampled so a track change crossfades from
 * the old colors rather than flashing through the static accent. No art, or art that
 * can't be read, resolves to the static accent pair for the current color scheme.
 */
export function useArtworkTint(artworkUrl: string | undefined, source: RailColorSource): TintPair {
  const colorScheme = useComputedColorScheme('dark');
  const fallback = STATIC_TINT[colorScheme];
  const [sampled, setSampled] = useState<TintPair | null>(null);

  const wantsArt = source === 'artwork' && !!artworkUrl;

  useEffect(() => {
    if (!wantsArt || !artworkUrl) return;
    let cancelled = false;
    void artworkTint(artworkUrl).then((tint) => {
      if (!cancelled) setSampled(tint);
    });
    return () => {
      cancelled = true;
    };
  }, [wantsArt, artworkUrl]);

  if (!wantsArt) return fallback;
  return sampled ?? fallback;
}
