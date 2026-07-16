import type { StellarStation } from '../types/stellarTunerLog';
import { normalizeText, normalizeTitle, stripTrailingTitleNumber } from './songMatcher';

export const SCROBBLE_THRESHOLD_MS = 25_000;
const RETRY_DELAYS_MS = [15_000, 60_000, 300_000] as const;

export interface ScrobblePayload {
  artist: string;
  title: string;
  album?: string;
  startedAt: number;
}

export interface ScrobbleSubmissionResult {
  accepted: boolean;
  ignoredCode: number;
  ignoredMessage: string | null;
}

export interface ProviderSubmissionError {
  code: number | null;
  message: string;
  retryable: boolean;
}

export interface ScrobbleProviderClient {
  id: string;
  updateNowPlaying: (track: Omit<ScrobblePayload, 'startedAt'>) => Promise<void>;
  scrobble: (track: ScrobblePayload) => Promise<ScrobbleSubmissionResult>;
  parseError: (error: unknown) => ProviderSubmissionError;
  onAuthenticationInvalid: (message: string) => void;
  onPermanentFailure?: (message: string) => void;
}

export interface PlaybackObservation {
  status: 'idle' | 'loading' | 'playing' | 'stopped' | 'error';
  channelId: number | null;
  station?: StellarStation;
}

interface Candidate {
  key: string;
  track: Omit<ScrobblePayload, 'startedAt'>;
  startedAt: number | null;
  accumulatedMs: number;
  activeSince: number | null;
  nowPlayingSent: boolean;
  eligible: boolean;
  thresholdTimer: ReturnType<typeof setTimeout> | null;
}

interface PendingScrobble {
  payload: ScrobblePayload;
  attempt: number;
}

interface ProviderState {
  active: boolean;
  client: ScrobbleProviderClient;
  candidate: Candidate | null;
  queue: PendingScrobble[];
  submitting: boolean;
  retryTimer: ReturnType<typeof setTimeout> | null;
}

function songObservation(channelId: number | null, station?: StellarStation) {
  if (channelId == null || !station || station.cut_type.trim().toLowerCase() !== 'song') return null;
  const artist = station.artist.trim();
  const title = stripTrailingTitleNumber(station.title);
  if (!artist || !title) return null;
  const album = station.album.trim() || undefined;
  return {
    key: `${channelId}|${normalizeText(artist)}|${normalizeTitle(title)}`,
    track: { artist, title, album },
  };
}

export class ScrobbleCoordinator {
  private providers = new Map<string, ProviderState>();

  update(observation: PlaybackObservation, activeProviders: ScrobbleProviderClient[]) {
    const activeIds = new Set(activeProviders.map((provider) => provider.id));
    for (const [id, state] of this.providers) {
      if (!activeIds.has(id)) {
        this.clearProvider(state);
        this.providers.delete(id);
      }
    }

    const song = songObservation(observation.channelId, observation.station);
    for (const client of activeProviders) {
      let state = this.providers.get(client.id);
      if (!state) {
        state = { active: true, client, candidate: null, queue: [], submitting: false, retryTimer: null };
        this.providers.set(client.id, state);
      } else {
        state.active = true;
        state.client = client;
      }
      this.updateProvider(state, song, observation.status === 'playing');
    }
  }

  dispose() {
    for (const state of this.providers.values()) this.clearProvider(state);
    this.providers.clear();
  }

  private updateProvider(
    state: ProviderState,
    song: ReturnType<typeof songObservation>,
    playing: boolean,
  ) {
    if (!song) {
      this.clearCandidate(state);
      return;
    }

    if (state.candidate?.key !== song.key) {
      this.clearCandidate(state);
      state.candidate = {
        key: song.key,
        track: song.track,
        startedAt: null,
        accumulatedMs: 0,
        activeSince: null,
        nowPlayingSent: false,
        eligible: false,
        thresholdTimer: null,
      };
    } else {
      state.candidate.track = song.track;
    }

    const candidate = state.candidate;
    if (!candidate) return;
    if (!playing) {
      this.pauseCandidate(candidate);
      return;
    }

    const now = Date.now();
    if (candidate.activeSince === null) candidate.activeSince = now;
    if (candidate.startedAt === null) candidate.startedAt = Math.floor(now / 1000);
    if (!candidate.nowPlayingSent) {
      candidate.nowPlayingSent = true;
      void state.client.updateNowPlaying(candidate.track).catch((error: unknown) => {
        const parsed = state.client.parseError(error);
        if (parsed.code === 9) state.client.onAuthenticationInvalid(parsed.message);
        else state.client.onPermanentFailure?.(`Last.fm now playing failed: ${parsed.message}`);
      });
    }
    this.scheduleThreshold(state, candidate);
  }

  private scheduleThreshold(state: ProviderState, candidate: Candidate) {
    if (candidate.eligible || candidate.thresholdTimer) return;
    const elapsed = candidate.accumulatedMs + (candidate.activeSince === null ? 0 : Date.now() - candidate.activeSince);
    const remaining = Math.max(0, SCROBBLE_THRESHOLD_MS - elapsed);
    candidate.thresholdTimer = setTimeout(() => {
      candidate.thresholdTimer = null;
      if (state.candidate !== candidate || candidate.activeSince === null || candidate.eligible) return;
      const now = Date.now();
      candidate.accumulatedMs += now - candidate.activeSince;
      candidate.activeSince = now;
      if (candidate.accumulatedMs < SCROBBLE_THRESHOLD_MS) {
        this.scheduleThreshold(state, candidate);
        return;
      }
      candidate.eligible = true;
    }, remaining);
  }

  private pauseCandidate(candidate: Candidate) {
    if (candidate.activeSince !== null) {
      candidate.accumulatedMs += Date.now() - candidate.activeSince;
      candidate.activeSince = null;
    }
    if (candidate.thresholdTimer) {
      clearTimeout(candidate.thresholdTimer);
      candidate.thresholdTimer = null;
    }
  }

  private clearCandidate(state: ProviderState) {
    const candidate = state.candidate;
    if (!candidate) return;
    this.pauseCandidate(candidate);
    if (!candidate.eligible && candidate.accumulatedMs >= SCROBBLE_THRESHOLD_MS) {
      candidate.eligible = true;
    }
    if (candidate.eligible && candidate.startedAt !== null) {
      state.queue.push({ payload: { ...candidate.track, startedAt: candidate.startedAt }, attempt: 0 });
      void this.processQueue(state);
    }
    state.candidate = null;
  }

  private clearProvider(state: ProviderState) {
    state.active = false;
    this.clearCandidate(state);
    state.queue = [];
    if (state.retryTimer) clearTimeout(state.retryTimer);
    state.retryTimer = null;
  }

  private async processQueue(state: ProviderState): Promise<void> {
    if (state.submitting || state.retryTimer || state.queue.length === 0) return;
    state.submitting = true;
    const pending = state.queue[0];
    try {
      const result = await state.client.scrobble(pending.payload);
      if (!state.active) return;
      state.queue.shift();
      if (!result.accepted) {
        state.client.onPermanentFailure?.(
          result.ignoredMessage || `Last.fm ignored the scrobble (code ${result.ignoredCode})`,
        );
      }
    } catch (error) {
      if (!state.active) return;
      const parsed = state.client.parseError(error);
      if (parsed.code === 9) {
        state.queue = [];
        state.client.onAuthenticationInvalid(parsed.message);
      } else if (parsed.retryable) {
        const delay = RETRY_DELAYS_MS[Math.min(pending.attempt, RETRY_DELAYS_MS.length - 1)];
        pending.attempt += 1;
        state.retryTimer = setTimeout(() => {
          state.retryTimer = null;
          void this.processQueue(state);
        }, delay);
      } else {
        state.queue.shift();
        state.client.onPermanentFailure?.(`Last.fm scrobble failed: ${parsed.message}`);
      }
    } finally {
      state.submitting = false;
    }
    if (!state.retryTimer && state.queue.length > 0) void this.processQueue(state);
  }
}
