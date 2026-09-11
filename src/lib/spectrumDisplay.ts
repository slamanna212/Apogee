/** Shape measured levels for a small display without inventing movement. */
export function spectrumDisplayLevels(levels: readonly number[], bands: number): number[] {
  if (bands <= 0) return [];
  if (levels.length === 0) return Array(bands).fill(0);

  const clean = levels.map((level) => Number.isFinite(level) ? Math.max(0, Math.min(1, level)) : 0);
  const mean = clean.reduce((sum, level) => sum + level, 0) / clean.length;
  // Loud, dense passages need headroom for their differences to remain visible.
  // Keep quieter passages level-sensitive, but limit the display's center.
  const center = Math.min(mean, 0.55);
  const shaped = clean.map((level) => {
    const contrast = Math.max(0, center + (level - mean) * 1.9);
    // Ease peaks toward the ceiling instead of clipping multiple bands to the
    // same height. Equal inputs stay equal; silence stays silent.
    const compressed = contrast <= 0.8
      ? contrast
      : 0.8 + 0.2 * (1 - Math.exp(-(contrast - 0.8) / 0.2));
    return Math.pow(compressed, 1.35);
  });

  return Array.from({ length: bands }, (_, i) => {
    // Pool the entire spectrum for compact layouts instead of discarding every
    // other band. Overlap weights also support non-integer band ratios.
    const start = i * shaped.length / bands;
    const end = (i + 1) * shaped.length / bands;
    let power = 0;
    for (let j = Math.floor(start); j < Math.ceil(end); j++) {
      const weight = Math.min(end, j + 1) - Math.max(start, j);
      power += (shaped[j] ?? 0) ** 2 * weight;
    }
    return Math.sqrt(power / (end - start));
  });
}
