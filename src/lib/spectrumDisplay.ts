/** Shape measured levels for a small display without inventing movement. */
export function spectrumDisplayLevels(levels: readonly number[], bands: number): number[] {
  if (bands <= 0) return [];
  if (levels.length === 0) return Array(bands).fill(0);

  const clean = levels.map((level) => Number.isFinite(level) ? Math.max(0, Math.min(1, level)) : 0);
  const mean = clean.reduce((sum, level) => sum + level, 0) / clean.length;
  const shaped = clean.map((level) => {
    // Expand differences around the measured average, then lower the quiet
    // background a little. Equal inputs stay equal; silence stays silent.
    const contrast = Math.max(0, Math.min(1, mean + (level - mean) * 1.9));
    return Math.pow(contrast, 1.35);
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
