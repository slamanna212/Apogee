import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { StellarStation } from '../types/stellarTunerLog';
import {
  ScrobbleCoordinator,
  type ProviderSubmissionError,
  type ScrobbleProviderClient,
} from './scrobbling';

function station(overrides: Partial<StellarStation> = {}): StellarStation {
  return {
    id: 'station',
    name: 'Channel',
    channel_number: 1,
    artist: 'Artist',
    title: 'Track (93)',
    album: 'Album',
    cut_type: 'Song',
    artwork_url: '',
    itunes_id: '',
    ...overrides,
  };
}

function provider() {
  const updateNowPlaying = vi.fn().mockResolvedValue(undefined);
  const scrobble = vi.fn().mockResolvedValue({ accepted: true, ignoredCode: 0, ignoredMessage: null });
  const onAuthenticationInvalid = vi.fn();
  const onPermanentFailure = vi.fn();
  const client: ScrobbleProviderClient = {
    id: 'test',
    updateNowPlaying,
    scrobble,
    parseError: (error) => error as ProviderSubmissionError,
    onAuthenticationInvalid,
    onPermanentFailure,
  };
  return { client, updateNowPlaying, scrobble, onAuthenticationInvalid, onPermanentFailure };
}

describe('ScrobbleCoordinator', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T00:00:00Z'));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('marks a track eligible after 25 seconds and scrobbles it once the app closes', async () => {
    const coordinator = new ScrobbleCoordinator();
    const test = provider();
    const current = station();
    coordinator.update({ status: 'playing', channelId: 1, station: current }, [test.client]);
    coordinator.update({ status: 'playing', channelId: 1, station: { ...current } }, [test.client]);

    expect(test.updateNowPlaying).toHaveBeenCalledTimes(1);
    expect(test.updateNowPlaying).toHaveBeenCalledWith({ artist: 'Artist', title: 'Track', album: 'Album' });
    await vi.advanceTimersByTimeAsync(24_999);
    expect(test.scrobble).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(test.scrobble).not.toHaveBeenCalled();
    coordinator.dispose();
    expect(test.scrobble).toHaveBeenCalledTimes(1);
    expect(test.scrobble).toHaveBeenCalledWith({
      artist: 'Artist',
      title: 'Track',
      album: 'Album',
      startedAt: 1767225600,
    });
  });

  it('excludes non-Song cuts from now playing and scrobbling', async () => {
    const coordinator = new ScrobbleCoordinator();
    const test = provider();
    coordinator.update({ status: 'playing', channelId: 1, station: station({ cut_type: 'Promo' }) }, [test.client]);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(test.updateNowPlaying).not.toHaveBeenCalled();
    expect(test.scrobble).not.toHaveBeenCalled();
    coordinator.dispose();
  });

  it('counts only active playback and resumes the same candidate', async () => {
    const coordinator = new ScrobbleCoordinator();
    const test = provider();
    const current = station();
    coordinator.update({ status: 'playing', channelId: 1, station: current }, [test.client]);
    await vi.advanceTimersByTimeAsync(10_000);
    coordinator.update({ status: 'stopped', channelId: 1, station: current }, [test.client]);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(test.scrobble).not.toHaveBeenCalled();
    coordinator.update({ status: 'playing', channelId: 1, station: current }, [test.client]);
    await vi.advanceTimersByTimeAsync(15_000);
    expect(test.scrobble).not.toHaveBeenCalled();
    coordinator.dispose();
    expect(test.scrobble).toHaveBeenCalledTimes(1);
  });

  it('resets an unfinished candidate when the channel or track changes', async () => {
    const coordinator = new ScrobbleCoordinator();
    const test = provider();
    coordinator.update({ status: 'playing', channelId: 1, station: station() }, [test.client]);
    await vi.advanceTimersByTimeAsync(20_000);
    coordinator.update({ status: 'playing', channelId: 2, station: station({ title: 'Other' }) }, [test.client]);
    await vi.advanceTimersByTimeAsync(5_000);
    expect(test.scrobble).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(20_000);
    expect(test.scrobble).not.toHaveBeenCalled();
    coordinator.update({ status: 'playing', channelId: 2, station: station({ title: 'Third' }) }, [test.client]);
    expect(test.scrobble).toHaveBeenCalledTimes(1);
    expect(test.scrobble.mock.calls[0][0].title).toBe('Other');
    coordinator.dispose();
  });

  it('retries transient scrobble failures while the process remains open', async () => {
    const coordinator = new ScrobbleCoordinator();
    const test = provider();
    test.scrobble
      .mockRejectedValueOnce({ code: 16, message: 'temporary', retryable: true })
      .mockResolvedValueOnce({ accepted: true, ignoredCode: 0, ignoredMessage: null });
    coordinator.update({ status: 'playing', channelId: 1, station: station() }, [test.client]);
    await vi.advanceTimersByTimeAsync(25_000);
    expect(test.scrobble).not.toHaveBeenCalled();
    coordinator.update({ status: 'playing', channelId: 2, station: station({ title: 'Other' }) }, [test.client]);
    expect(test.scrobble).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(14_999);
    expect(test.scrobble).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(test.scrobble).toHaveBeenCalledTimes(2);
    coordinator.dispose();
  });
});
