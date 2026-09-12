export interface AudioBufferSettings {
  capacityMs: number;
  startMs: number;
  rebufferMs: number;
}

export const DEFAULT_AUDIO_BUFFER: AudioBufferSettings = {
  capacityMs: 5000,
  startMs: 1000,
  rebufferMs: 150,
};

export function audioBufferError(value: AudioBufferSettings): string | null {
  if (![value.capacityMs, value.startMs, value.rebufferMs].every(Number.isInteger)) return 'Enter whole milliseconds for each value.';
  if (value.capacityMs < 100 || value.capacityMs > 10000) return 'Buffer capacity must be between 100 and 10,000 ms.';
  if (value.startMs < 50 || value.startMs > value.capacityMs) return 'Startup buffer must be at least 50 ms and no greater than capacity.';
  if (value.rebufferMs < 0 || value.rebufferMs >= value.startMs) return 'Rebuffer threshold must be at least 0 ms and less than the startup buffer.';
  return null;
}

export function normalizeAudioBuffer(value: unknown): AudioBufferSettings {
  const candidate = { ...DEFAULT_AUDIO_BUFFER, ...(typeof value === 'object' && value !== null ? value : {}) };
  // Upgrade the previous shipped defaults; retain custom tuning.
  const previousDefaults = candidate.capacityMs === 2000 && candidate.startMs === 500 && candidate.rebufferMs === 150;
  return audioBufferError(candidate) || previousDefaults ? { ...DEFAULT_AUDIO_BUFFER } : candidate;
}
