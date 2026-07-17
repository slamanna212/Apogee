import type { AlertEntry } from '../types/alerts';

export interface PersistedAlertsData {
  entries: AlertEntry[];
  notifyOS: boolean;
  notifyInApp: boolean;
}

const DEFAULT_ALERTS: PersistedAlertsData = {
  entries: [],
  notifyOS: true,
  notifyInApp: true,
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isAlertEntry(value: unknown): value is AlertEntry {
  if (!isRecord(value)) return false;
  if (typeof value.id !== 'string' || value.id.length === 0) return false;
  if (value.type !== 'artist' && value.type !== 'track') return false;
  if (typeof value.artist !== 'string' || value.artist.trim().length === 0) return false;
  if (value.type === 'track' && (typeof value.title !== 'string' || value.title.trim().length === 0)) return false;
  return typeof value.createdAt === 'number' && Number.isFinite(value.createdAt);
}

/** Validates data read from alerts.json before exposing it to React. */
export function sanitizePersistedAlerts(value: unknown): PersistedAlertsData {
  if (!isRecord(value)) return { ...DEFAULT_ALERTS };
  return {
    entries: Array.isArray(value.entries) ? value.entries.filter(isAlertEntry) : [],
    notifyOS: typeof value.notifyOS === 'boolean' ? value.notifyOS : DEFAULT_ALERTS.notifyOS,
    notifyInApp: typeof value.notifyInApp === 'boolean' ? value.notifyInApp : DEFAULT_ALERTS.notifyInApp,
  };
}
