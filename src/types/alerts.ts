export type AlertEntryType = 'track' | 'artist';

export interface AlertEntry {
  id: string;
  type: AlertEntryType;
  artist: string;
  /** Only meaningful when type === 'track'. */
  title?: string;
  createdAt: number;
}
