import { invoke } from '@tauri-apps/api/core';
import type {
  LastFmAuthStart,
  LastFmConnectionStatus,
  LastFmError,
  LastFmScrobble,
  LastFmScrobbleResult,
  LastFmTrack,
} from '../types/lastfm';

export function getLastFmConnectionStatus(): Promise<LastFmConnectionStatus> {
  return invoke('lastfm_connection_status');
}

export function beginLastFmAuth(): Promise<LastFmAuthStart> {
  return invoke('lastfm_begin_auth');
}

export function completeLastFmAuth(token: string): Promise<LastFmConnectionStatus> {
  return invoke('lastfm_complete_auth', { token });
}

export function disconnectLastFm(): Promise<void> {
  return invoke('lastfm_disconnect');
}

export function updateLastFmNowPlaying(track: LastFmTrack): Promise<void> {
  return invoke('lastfm_update_now_playing', { track });
}

export function scrobbleLastFm(scrobble: LastFmScrobble): Promise<LastFmScrobbleResult> {
  return invoke('lastfm_scrobble', { scrobble });
}

export function asLastFmError(error: unknown): LastFmError {
  if (typeof error === 'object' && error !== null) {
    const candidate = error as Partial<LastFmError>;
    if (typeof candidate.message === 'string') {
      return {
        code: typeof candidate.code === 'number' ? candidate.code : null,
        message: candidate.message,
        retryable: candidate.retryable === true,
      };
    }
  }
  return {
    code: null,
    message: error instanceof Error ? error.message : String(error),
    retryable: false,
  };
}
