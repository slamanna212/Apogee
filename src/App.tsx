import { lazy, Suspense, useCallback, useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { MantineProvider } from '@mantine/core';
import { Notifications, notifications } from '@mantine/notifications';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { currentMonitor, primaryMonitor } from '@tauri-apps/api/window';
import { PhysicalPosition } from '@tauri-apps/api/dpi';
import { IconApps, IconBell, IconHistory, IconHome2, IconMinus, IconSettings, IconSquare, IconStar, IconX } from '@tabler/icons-react';
import { error as logError, warn as logWarn } from '@tauri-apps/plugin-log';
import logoUrl from './assets/logo.svg';
import { theme, cssVariablesResolver } from './theme';
import { useSettingsStore } from './stores/settingsStore';
import { nextPollDelayMs, useChannelStore } from './stores/channelStore';
import { usePlayerStore } from './stores/playerStore';
import { useLibraryStore } from './stores/libraryStore';
import { useUpdateStore } from './stores/updateStore';
import { useAlertsStore } from './stores/alertsStore';
import { useScrobblingStore } from './stores/scrobblingStore';
import { useSleepTimerStore } from './stores/sleepTimerStore';
import { setMediaMetadata } from './lib/mediaSession';
import { setWaveformDevice } from './lib/waveform';
import {
  discordRpcConnect,
  discordRpcDisconnect,
  discordRpcSetActivity,
  discordRpcClearActivity,
  resolveDiscordActivity,
} from './lib/discordRpc';
import { TransportBar, type BarMode } from './components/TransportBar';
import { ChannelModal } from './components/ChannelModal';
import { UpdateModal } from './components/UpdateModal';
import { Home } from './pages/Home';
import { Channels } from './pages/Channels';
import { Recent } from './pages/Recent';
import { Favorites } from './pages/Favorites';

import { asLastFmError, scrobbleLastFm, updateLastFmNowPlaying } from './lib/lastfm';
import { ScrobbleCoordinator, type ScrobbleProviderClient } from './lib/scrobbling';

// Not needed on first paint (onboarding only runs once; Alerts/Settings are
// secondary pages), so keep them out of the main bundle.
const OnboardingWizard = lazy(() =>
  import('./components/onboarding/OnboardingWizard').then((m) => ({ default: m.OnboardingWizard })),
);
const Alerts = lazy(() => import('./pages/Alerts').then((m) => ({ default: m.Alerts })));
const Settings = lazy(() => import('./pages/Settings').then((m) => ({ default: m.Settings })));

type Page = 'home' | 'channels' | 'recent' | 'favorites' | 'alerts' | 'settings';

const NAV_ITEMS: { page: Page; label: string; icon: typeof IconHome2 }[] = [
  { page: 'home', label: 'Home', icon: IconHome2 },
  { page: 'channels', label: 'Channels', icon: IconApps },
  { page: 'recent', label: 'Recent', icon: IconHistory },
  { page: 'favorites', label: 'Favorites', icon: IconStar },
  { page: 'alerts', label: 'Alerts', icon: IconBell },
];

const COMPACT_BREAKPOINT = 900;
const CARD_WIDTH = 1180;
const CARD_HEIGHT = 760;
const RAIL_SHADOW_GUTTER = 24;
const EXPANDED_RAIL_SIZE = { width: 700, height: 84 };
const COLLAPSED_RAIL_SIZE = { width: 300, height: 56 };
const EXPANDED_BAR_SIZE = {
  width: EXPANDED_RAIL_SIZE.width + RAIL_SHADOW_GUTTER * 2,
  height: EXPANDED_RAIL_SIZE.height + RAIL_SHADOW_GUTTER * 2,
};
const COLLAPSED_BAR_SIZE = {
  width: COLLAPSED_RAIL_SIZE.width + RAIL_SHADOW_GUTTER * 2,
  height: COLLAPSED_RAIL_SIZE.height + RAIL_SHADOW_GUTTER * 2,
};
/** Align the expanded rail's center with the browser card's bottom edge. */
const BAR_OVERLAP = EXPANDED_RAIL_SIZE.height / 2 + RAIL_SHADOW_GUTTER;

const LASTFM_PROVIDER_CLIENT: ScrobbleProviderClient = {
  id: 'lastfm',
  updateNowPlaying: updateLastFmNowPlaying,
  scrobble: scrobbleLastFm,
  parseError: asLastFmError,
  onAuthenticationInvalid(message) {
    useScrobblingStore.getState().markLastFmDisconnected(message);
  },
  onPermanentFailure(message) {
    void logWarn(message);
  },
};

/**
 * One real OS window throughout. The "browser" is a rounded card absolutely
 * positioned inside it; the transport bar is a second absolutely-positioned
 * layer pinned to the window's bottom edge, overlapping the card by
 * BAR_OVERLAP so it reads as floating on top of it (per the design spec),
 * exactly like a single physical object rather than two windows that merely
 * track each other.
 */
async function applyWindowState(
  browserOpen: boolean,
  barMode: BarMode,
  overridePosition?: PhysicalPosition | null,
) {
  const win = getCurrentWebviewWindow();
  const monitor = (await currentMonitor()) ?? (await primaryMonitor());
  const scale = monitor?.scaleFactor ?? 1;
  const keepMiniWindowOnTop = useSettingsStore.getState().settings.keepMiniWindowOnTop;

  const target = browserOpen
    ? { width: CARD_WIDTH, height: CARD_HEIGHT + BAR_OVERLAP, resizable: true, alwaysOnTop: false }
    : barMode === 'expanded'
      ? { ...EXPANDED_BAR_SIZE, resizable: false, alwaysOnTop: keepMiniWindowOnTop }
      : { ...COLLAPSED_BAR_SIZE, resizable: false, alwaysOnTop: keepMiniWindowOnTop };

  const width = Math.round(target.width * scale);
  const height = Math.round(target.height * scale);

  // Keep the transport's bottom-center point fixed on screen while the native
  // window changes shape. Resizing first clips the bottom-pinned transport out
  // of the old window, then moving the window makes it reappear elsewhere. By
  // deriving the new origin from the current bounds and submitting both native
  // operations together, the browser instead grows/collapses around one
  // continuously mounted player.
  const [currentPosition, currentSize] = await Promise.all([
    overridePosition ? Promise.resolve(overridePosition) : win.outerPosition(),
    win.outerSize(),
  ]);
  const anchorX = currentPosition.x + currentSize.width / 2;
  const anchorY = currentPosition.y + currentSize.height;
  let x = Math.round(anchorX - width / 2);
  let y = Math.round(anchorY - height);

  if (monitor) {
    const area = monitor.workArea;
    const maxX = area.position.x + area.size.width - width;
    const maxY = area.position.y + area.size.height - height;
    x = maxX >= area.position.x
      ? Math.min(Math.max(x, area.position.x), maxX)
      : Math.round(area.position.x + (area.size.width - width) / 2);
    y = maxY >= area.position.y
      ? Math.min(Math.max(y, area.position.y), maxY)
      : Math.round(area.position.y + (area.size.height - height) / 2);
  }

  // On Linux/GTK, calling setResizable(false) before setSize locks the
  // window manager's min/max size hints to whatever size the window
  // currently is, which can leave setSize's request only partially applied
  // (the OS-level window - and thus its WM drag/placement bounds - stays
  // close to the old, larger size even though content renders smaller).
  // Always resize while resizable, then lock resizability down afterward.
  await win.setResizable(true);
  await invoke('set_window_bounds', { x, y, width, height });

  await win.setResizable(target.resizable);
  await win.setAlwaysOnTop(target.alwaysOnTop);
}

// macOS convention puts window controls at the left of the titlebar
// (traffic lights); Windows/Linux put them at the right.
const isMac = navigator.userAgent.includes('Mac');

const titlebarBtnStyle: CSSProperties = {
  width: 44,
  height: 36,
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  cursor: 'pointer',
  color: 'var(--app-dim)',
};

function navItemStyle(active: boolean, compact: boolean): CSSProperties {
  return {
    display: 'flex',
    alignItems: 'center',
    gap: 10,
    padding: '11px 12px',
    borderRadius: 12,
    cursor: 'pointer',
    background: active ? 'var(--app-accent-soft)' : 'transparent',
    color: active ? 'var(--app-text)' : 'var(--app-dim)',
    font: active ? '600 14px "Sora", sans-serif' : '500 14px "Sora", sans-serif',
    justifyContent: compact ? 'center' : 'flex-start',
  };
}

function TitlebarButton({ onClick, label, children }: { onClick: () => void; label: string; children: ReactNode }) {
  return (
    <div onClick={onClick} role="button" aria-label={label} style={titlebarBtnStyle}>
      {children}
    </div>
  );
}

function AppContent() {
  const { settings, builtinStellarApiKey, loaded: settingsLoaded, load: loadSettings } = useSettingsStore();
  const stellarApiKey = builtinStellarApiKey ?? '';
  // Per-field selectors (not a whole-store destructure) so this large shell
  // re-renders only on the slices it actually uses - not on every bitrate/poll
  // update pushed into these stores. Actions are stable store references.
  const channels = useChannelStore((s) => s.channels);
  const channelMetadata = useChannelStore((s) => s.channelMetadata);
  const nowPlaying = useChannelStore((s) => s.nowPlaying);
  const fetchChannels = useChannelStore((s) => s.fetchChannels);
  const pollNowPlaying = useChannelStore((s) => s.pollNowPlaying);
  const fetchChannelMetadata = useChannelStore((s) => s.fetchChannelMetadata);
  const playerStatus = usePlayerStore((s) => s.status);
  const currentChannel = usePlayerStore((s) => s.currentChannel);
  const volume = usePlayerStore((s) => s.volume);
  const muted = usePlayerStore((s) => s.muted);
  const errorMessage = usePlayerStore((s) => s.errorMessage);
  const isBuffering = usePlayerStore((s) => s.isBuffering);
  const selectChannel = usePlayerStore((s) => s.selectChannel);
  const play = usePlayerStore((s) => s.play);
  const stop = usePlayerStore((s) => s.stop);
  const setVolume = usePlayerStore((s) => s.setVolume);
  const toggleMute = usePlayerStore((s) => s.toggleMute);
  const initEventListener = usePlayerStore((s) => s.initEventListener);
  const { loaded: libraryLoaded, load: loadLibrary, recordPlay, favorites, toggleFavorite } = useLibraryStore();
  const lastFmConnection = useScrobblingStore((state) => state.providers.lastfm);
  const scrobbleCoordinatorRef = useRef<ScrobbleCoordinator | null>(null);
  if (!scrobbleCoordinatorRef.current) scrobbleCoordinatorRef.current = new ScrobbleCoordinator();

  const [browserOpen, setBrowserOpen] = useState(true);
  const [barMode, setBarMode] = useState<BarMode>('expanded');
  const [page, setPage] = useState<Page>('home');
  const [modalStreamId, setModalStreamId] = useState<number | null>(null);
  const [compact, setCompact] = useState(false);

  const onboardingActive = settingsLoaded && !settings.onboardingComplete;
  const updateStatus = useUpdateStore((s) => s.status);
  const updateModalActive = updateStatus !== 'idle' && updateStatus !== 'checking';

  // Stable references so the memoized ChannelCard/ChannelListRow (see
  // components/ChannelCard.tsx, ChannelListRow.tsx) can actually skip
  // re-rendering when passed these as onClick/onInfo - a plain function
  // declaration recreated every render would defeat that memoization.
  const handleOpenChannel = useCallback((streamId: number) => {
    setModalStreamId(streamId);
  }, []);

  const handlePlayChannel = useCallback(
    async (streamId: number) => {
      const channel = channels.find((c) => c.stream_id === streamId);
      if (!channel) return;
      // Switching channels shouldn't leave a timer armed from the previous one.
      useSleepTimerStore.getState().cancel();
      try {
        await selectChannel(
          channel,
          { baseUrl: settings.baseUrl, username: settings.username, password: settings.password },
        );
        if (libraryLoaded) recordPlay(channel.stream_id);
      } catch (err) {
        logError(`playback failed: ${err instanceof Error ? err.message : String(err)}`);
      }
    },
    [channels, settings.baseUrl, settings.username, settings.password, selectChannel, libraryLoaded, recordPlay],
  );

  useEffect(() => {
    if (onboardingActive && !browserOpen) setBrowserOpen(true);
  }, [onboardingActive, browserOpen]);

  // The update modal (~440px) is portaled into #apogee-window, which is
  // clipped to whatever shape the OS window currently is. In mini-player
  // mode that's a small pill-shaped window, so the modal's overlay/shadow
  // get clipped hard against those tiny rounded bounds - it shows up as
  // stray shadow bands rather than a proper modal. Force the full card open
  // so the modal has room to render as intended.
  useEffect(() => {
    if (updateModalActive && !browserOpen) setBrowserOpen(true);
  }, [updateModalActive, browserOpen]);

  useEffect(() => {
    loadSettings();
    loadLibrary();
    void useAlertsStore.getState().load();
    void useScrobblingStore.getState().load();
    initEventListener();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => () => scrobbleCoordinatorRef.current?.dispose(), []);

  const handlePlayChannelRef = useRef(handlePlayChannel);
  useEffect(() => {
    handlePlayChannelRef.current = handlePlayChannel;
  });

  useEffect(() => {
    let unregister: (() => void) | undefined;
    listen<number>('notification-tune', ({ payload: streamId }) => {
      if (typeof streamId === 'number') void handlePlayChannelRef.current(streamId);
    }).then((unlisten) => {
      unregister = unlisten;
    });

    return () => unregister?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (settingsLoaded) {
      usePlayerStore.setState({ volume: settings.volume });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settingsLoaded]);

  useEffect(() => {
    if (settingsLoaded) {
      invoke('set_log_level', { verbose: settings.verboseLogging }).catch(() => {});
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settingsLoaded]);

  // Point the visualizer at the saved output device from launch (its capture
  // runs independently of playback). Changes made in Settings apply themselves;
  // this only restores the persisted choice on startup.
  useEffect(() => {
    if (settingsLoaded) {
      const device = settings.audioDevice;
      void setWaveformDevice(device?.name ?? null, device?.description ?? null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settingsLoaded]);

  useEffect(() => {
    if (!settingsLoaded) return;
    if (settings.discordRpcEnabled) {
      void discordRpcConnect();
    } else {
      void discordRpcDisconnect();
    }
  }, [settingsLoaded, settings.discordRpcEnabled]);

  useEffect(() => {
    if (!settingsLoaded) return;
    const timer = setTimeout(() => {
      void useUpdateStore.getState().checkForUpdates(settings.updateChannel);
    }, 5000);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settingsLoaded]);

  useEffect(() => {
    if (settingsLoaded && settings.baseUrl && settings.username && settings.categoryId) {
      fetchChannels(
        { baseUrl: settings.baseUrl, username: settings.username, password: settings.password },
        settings.categoryId,
      );
    }
  }, [settingsLoaded, settings.baseUrl, settings.username, settings.password, settings.categoryId, fetchChannels]);

  useEffect(() => {
    if (!settingsLoaded || channels.length === 0) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;

    async function tick() {
      await pollNowPlaying(stellarApiKey);
      if (cancelled) return;
      // Reschedule using the failure count pollNowPlaying just updated, so a
      // StellarTunerLog outage backs off instead of polling at a fixed rate.
      const delay = nextPollDelayMs(useChannelStore.getState().pollFailureCount);
      timer = setTimeout(tick, delay);
    }

    void tick();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [settingsLoaded, stellarApiKey, channels, pollNowPlaying]);

  useEffect(() => {
    if (channels.length > 0) fetchChannelMetadata();
  }, [channels, fetchChannelMetadata]);

  useEffect(() => {
    useAlertsStore.getState().scan(nowPlaying, browserOpen, handlePlayChannel);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nowPlaying, browserOpen]);

  useEffect(() => {
    if (!browserOpen) notifications.clean();
  }, [browserOpen]);

  const currentNowPlaying = currentChannel ? nowPlaying.get(currentChannel.stream_id) : undefined;

  useEffect(() => {
    const enabled = settings.scrobbling.lastfm.enabled && lastFmConnection.connected;
    scrobbleCoordinatorRef.current?.update(
      {
        status: playerStatus,
        channelId: currentChannel?.stream_id ?? null,
        station: currentNowPlaying,
      },
      enabled ? [LASTFM_PROVIDER_CLIENT] : [],
    );
  }, [
    playerStatus,
    currentChannel,
    currentNowPlaying,
    settings.scrobbling.lastfm.enabled,
    lastFmConnection.connected,
  ]);

  useEffect(() => {
    if (!currentChannel) return;
    if (currentNowPlaying) {
      setMediaMetadata({
        title: currentNowPlaying.title,
        artist: currentNowPlaying.artist,
        album: currentNowPlaying.album || undefined,
        coverUrl: currentNowPlaying.artwork_url || currentChannel.stream_icon,
      });
    } else {
      setMediaMetadata({ title: currentChannel.name, artist: '', coverUrl: currentChannel.stream_icon });
    }
  }, [currentChannel, currentNowPlaying]);

  useEffect(() => {
    if (!settings.discordRpcEnabled) return;
    const activity = resolveDiscordActivity(playerStatus, currentChannel, currentNowPlaying);
    if (activity) {
      void discordRpcSetActivity(activity);
    } else {
      void discordRpcClearActivity();
    }
  }, [playerStatus, currentChannel, currentNowPlaying, settings.discordRpcEnabled]);

  useEffect(() => {
    void applyWindowState(browserOpen, barMode);
  }, [browserOpen, barMode, settings.keepMiniWindowOnTop]);

  const windowStateRef = useRef({ browserOpen, barMode });
  useEffect(() => {
    windowStateRef.current = { browserOpen, barMode };
  }, [browserOpen, barMode]);

  const wasMinimizedRef = useRef(false);
  const lastKnownPositionRef = useRef<PhysicalPosition | null>(null);
  useEffect(() => {
    // On Linux, undecorated/transparent windows sometimes get left at the
    // wrong OS size after being minimized and restored (a GTK/WM quirk, not
    // something the app's own state tracks). Regaining focus is a reliable
    // signal that the window just came back, so re-assert the size for the
    // current mode at that point, and restore the window to wherever it
    // actually was (tracked via onMoved) rather than recomputing a
    // canonical centered/docked position - otherwise every minimize/restore
    // would discard a manual drag and snap the window back to that spot.
    //
    // Only do this for an actual minimize -> restore transition, not every
    // focus-changed event: dragging via data-tauri-drag-region is known to
    // fire spurious focus toggles mid-drag (tauri-apps/tauri#10767), and
    // unconditionally re-asserting position on every focus regain fights
    // the live OS-driven drag, making the window feel like it snaps/can't
    // be moved past a point.
    const win = getCurrentWebviewWindow();
    let unlistenFocus: (() => void) | undefined;
    let unlistenMoved: (() => void) | undefined;

    win
      .onMoved(({ payload }) => {
        lastKnownPositionRef.current = payload;
      })
      .then((fn) => {
        unlistenMoved = fn;
      });

    win
      .onFocusChanged(async ({ payload: focused }) => {
        const minimized = await win.isMinimized();
        const wasMinimized = wasMinimizedRef.current;
        wasMinimizedRef.current = minimized;
        if (!focused || minimized || !wasMinimized) return;
        const { browserOpen, barMode } = windowStateRef.current;
        void applyWindowState(browserOpen, barMode, lastKnownPositionRef.current);
      })
      .then((fn) => {
        unlistenFocus = fn;
      });

    return () => {
      unlistenFocus?.();
      unlistenMoved?.();
    };
  }, []);

  useEffect(() => {
    function handleResize() {
      setCompact(window.innerWidth < COMPACT_BREAKPOINT);
    }
    handleResize();
    window.addEventListener('resize', handleResize);
    return () => window.removeEventListener('resize', handleResize);
  }, []);

  function handlePlus() {
    if (!browserOpen && barMode === 'collapsed') {
      setBarMode('expanded');
    } else if (!browserOpen && barMode === 'expanded') {
      setBrowserOpen(true);
    }
  }

  function handleMinus() {
    if (browserOpen) {
      setBrowserOpen(false);
    } else if (barMode === 'expanded') {
      setBarMode('collapsed');
    }
  }

  const modalChannel = modalStreamId != null ? channels.find((c) => c.stream_id === modalStreamId) : undefined;
  const win = getCurrentWebviewWindow();

  return (
    <div
      id="apogee-window"
      style={{
        position: 'relative',
        width: '100%',
        height: '100%',
        overflow: 'hidden',
        // The transport surface owns the mini-player's capsule shape. Applying
        // another extreme radius here clips the surface asymmetrically and can
        // cover controls near the right edge.
        borderRadius: browserOpen ? 26 : 0,
        transform: 'translateZ(0)',
      }}
    >
      {browserOpen && (
        <div
          style={{
            position: 'absolute',
            top: 0,
            left: 0,
            right: 0,
            bottom: BAR_OVERLAP,
            borderRadius: 26,
            overflow: 'hidden',
            border: '1px solid var(--app-border)',
            boxShadow: 'var(--app-shadow-card)',
            display: 'flex',
            flexDirection: 'column',
            background: 'var(--app-bg2)',
          }}
        >
          <div
            data-tauri-drag-region
            style={{
              height: 36,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              flex: 'none',
              flexDirection: isMac ? 'row-reverse' : 'row',
            }}
          >
            <img
              src={logoUrl}
              alt=""
              width={18}
              height={18}
              style={{ marginLeft: isMac ? 0 : 12, marginRight: isMac ? 12 : 0 }}
            />
            <div style={{ display: 'flex' }}>
              {isMac ? (
                <>
                  <TitlebarButton label="Quit" onClick={() => win.close()}>
                    <IconX size={14} />
                  </TitlebarButton>
                  <TitlebarButton label="Minimize" onClick={() => win.minimize()}>
                    <IconMinus size={14} />
                  </TitlebarButton>
                  <TitlebarButton label="Maximize" onClick={() => win.toggleMaximize()}>
                    <IconSquare size={12} />
                  </TitlebarButton>
                </>
              ) : (
                <>
                  <TitlebarButton label="Minimize" onClick={() => win.minimize()}>
                    <IconMinus size={14} />
                  </TitlebarButton>
                  <TitlebarButton label="Maximize" onClick={() => win.toggleMaximize()}>
                    <IconSquare size={12} />
                  </TitlebarButton>
                  <TitlebarButton label="Quit" onClick={() => win.close()}>
                    <IconX size={14} />
                  </TitlebarButton>
                </>
              )}
            </div>
          </div>

          <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
            {onboardingActive ? (
              <Suspense fallback={null}>
                <OnboardingWizard />
              </Suspense>
            ) : (
              <>
                <div
                  style={{
                    flex: 'none',
                    width: compact ? 68 : 220,
                    background: 'rgba(255,255,255,.045)',
                    borderRight: '1px solid var(--app-border)',
                    padding: '0 16px 26px',
                    display: 'flex',
                    flexDirection: 'column',
                    gap: 4,
                  }}
                >
                  <div
                    style={{
                      height: 40,
                      marginBottom: 6,
                      display: 'flex',
                      alignItems: 'center',
                      gap: 8,
                      justifyContent: compact ? 'center' : 'flex-start',
                    }}
                  >
                    <img src={logoUrl} alt="Apogee" width={24} height={24} />
                    {!compact && <span style={{ font: '700 15px "Space Grotesk", sans-serif', color: 'var(--app-text)' }}>Apogee</span>}
                  </div>
                  {NAV_ITEMS.map(({ page: p, label, icon: Icon }) => (
                    <div key={p} onClick={() => setPage(p)} role="button" style={navItemStyle(page === p, compact)}>
                      <Icon size={17} />
                      {!compact && label}
                    </div>
                  ))}
                  <div style={{ flex: 1 }} />
                  <div onClick={() => setPage('settings')} role="button" style={navItemStyle(page === 'settings', compact)}>
                    <IconSettings size={17} />
                    {!compact && 'Settings'}
                  </div>
                </div>

                <div
                  style={{
                    flex: 1,
                    background: 'radial-gradient(circle at 30% 0%, var(--app-accent-soft), transparent 55%), var(--app-bg2)',
                    padding: '28px 32px',
                    overflowY: 'auto',
                    overflowX: 'hidden',
                    display: 'flex',
                    flexDirection: 'column',
                    color: 'var(--app-text)',
                  }}
                >
                  {page === 'home' && <Home onSelectChannel={handleOpenChannel} onPlayChannel={handlePlayChannel} />}
                  {page === 'channels' && <Channels onSelectChannel={handleOpenChannel} onPlayChannel={handlePlayChannel} />}
                  {page === 'recent' && <Recent onSelectChannel={handleOpenChannel} onPlayChannel={handlePlayChannel} />}
                  {page === 'favorites' && <Favorites onSelectChannel={handleOpenChannel} onPlayChannel={handlePlayChannel} />}
                  {page === 'alerts' && (
                    <Suspense fallback={null}>
                      <Alerts onPlayChannel={handlePlayChannel} />
                    </Suspense>
                  )}
                  {page === 'settings' && (
                    <Suspense fallback={null}>
                      <Settings />
                    </Suspense>
                  )}
                </div>
              </>
            )}
          </div>
        </div>
      )}

      {!onboardingActive && (
        <div
          style={{
            position: 'absolute',
            left: '50%',
            bottom: RAIL_SHADOW_GUTTER,
            width: barMode === 'expanded' ? EXPANDED_RAIL_SIZE.width : COLLAPSED_RAIL_SIZE.width,
            height: barMode === 'expanded' ? EXPANDED_RAIL_SIZE.height : COLLAPSED_RAIL_SIZE.height,
            transform: 'translateX(-50%)',
            zIndex: 10,
          }}
        >
          <TransportBar
            mode={barMode}
            status={playerStatus}
            currentChannel={currentChannel}
            channelMetadata={currentChannel ? channelMetadata.get(currentChannel.stream_id) : undefined}
            nowPlaying={currentNowPlaying}
            volume={volume}
            isFavorite={currentChannel ? favorites.includes(currentChannel.stream_id) : false}
            onToggleFavorite={() => currentChannel && toggleFavorite(currentChannel.stream_id)}
            onPlus={handlePlus}
            onMinus={handleMinus}
            errorMessage={errorMessage}
            isBuffering={isBuffering}
            onPlayStop={() => {
              const stopping = playerStatus === 'playing' || playerStatus === 'loading';
              if (stopping) useSleepTimerStore.getState().cancel();
              const action = stopping ? stop() : play();
              action.catch((err) => logError(`play/stop failed: ${err instanceof Error ? err.message : String(err)}`));
            }}
            onVolumeChange={setVolume}
            muted={muted}
            onToggleMute={toggleMute}
            compactVolumePopover={!browserOpen}
            isMiniPlayer={!browserOpen}
          />
        </div>
      )}

      {modalChannel && (
        <ChannelModal
          channel={modalChannel}
          metadata={channelMetadata.get(modalChannel.stream_id)}
          apiKey={stellarApiKey}
          isFavorite={favorites.includes(modalChannel.stream_id)}
          onToggleFavorite={() => toggleFavorite(modalChannel.stream_id)}
          onClose={() => setModalStreamId(null)}
        />
      )}

      {!onboardingActive && <UpdateModal />}
    </div>
  );
}

function useSystemColorScheme(): 'light' | 'dark' {
  const [scheme, setScheme] = useState<'light' | 'dark'>(() =>
    window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
  );

  useEffect(() => {
    const query = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = (e: MediaQueryListEvent) => setScheme(e.matches ? 'dark' : 'light');
    query.addEventListener('change', handler);
    return () => query.removeEventListener('change', handler);
  }, []);

  return scheme;
}

function App() {
  const themeMode = useLibraryStore((s) => s.themeMode);
  const systemScheme = useSystemColorScheme();
  const resolvedScheme = themeMode === 'system' ? systemScheme : themeMode;

  useEffect(() => {
    document.body.style.background = 'transparent';
  }, []);

  return (
    <MantineProvider theme={theme} cssVariablesResolver={cssVariablesResolver} forceColorScheme={resolvedScheme}>
      <Notifications position="top-right" />
      <AppContent />
    </MantineProvider>
  );
}

export default App;
