import type { XtreamChannel } from './xtream';

export interface PlayerState {
  status: 'idle' | 'loading' | 'playing' | 'paused' | 'error';
  currentChannel: XtreamChannel | null;
  volume: number;
  bitrateKbps: number | null;
}
