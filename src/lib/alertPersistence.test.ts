import { describe, expect, it } from 'vitest';
import { sanitizePersistedAlerts } from './alertPersistence';

const validTrack = {
  id: 'track-1',
  type: 'track',
  artist: 'Artist',
  title: 'Title',
  createdAt: 123,
};

describe('sanitizePersistedAlerts', () => {
  it('preserves valid persisted settings and entries', () => {
    expect(sanitizePersistedAlerts({ entries: [validTrack], notifyOS: false, notifyInApp: true })).toEqual({
      entries: [validTrack],
      notifyOS: false,
      notifyInApp: true,
    });
  });

  it('uses safe defaults when the persisted alerts value is malformed', () => {
    expect(sanitizePersistedAlerts(null)).toEqual({ entries: [], notifyOS: true, notifyInApp: true });
    expect(sanitizePersistedAlerts({ entries: null, notifyOS: 'yes' })).toEqual({
      entries: [],
      notifyOS: true,
      notifyInApp: true,
    });
  });

  it('drops malformed entries without discarding valid follows', () => {
    const result = sanitizePersistedAlerts({
      entries: [validTrack, { ...validTrack, id: 'bad', artist: null }, { surprise: true }],
    });
    expect(result.entries).toEqual([validTrack]);
  });
});
