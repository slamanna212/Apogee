export interface LastFmError {
  code: number | null;
  message: string;
  retryable: boolean;
}

export interface LastFmConnectionStatus {
  available: boolean;
  connected: boolean;
  username: string | null;
}

export interface LastFmAuthStart {
  token: string;
  authorizationUrl: string;
}

export interface LastFmTrack {
  artist: string;
  title: string;
  album?: string;
}

export interface LastFmScrobble extends LastFmTrack {
  startedAt: number;
}

export interface LastFmScrobbleResult {
  accepted: boolean;
  ignoredCode: number;
  ignoredMessage: string | null;
}
