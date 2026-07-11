import type { XtreamChannel } from './xtream';

export interface PlayerState {
  status: 'idle' | 'loading' | 'playing' | 'stopped' | 'error';
  currentChannel: XtreamChannel | null;
  volume: number;
  bitrateKbps: number | null;
}
