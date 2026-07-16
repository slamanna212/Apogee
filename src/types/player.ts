import type { XtreamChannel } from './xtream';

export type PlayerStatus = 'idle' | 'loading' | 'playing' | 'stopped' | 'error';

export interface PlayerState {
  status: PlayerStatus;
  currentChannel: XtreamChannel | null;
  volume: number;
  muted: boolean;
  bitrateKbps: number | null;
  errorMessage: string | null;
  isBuffering: boolean;
}
