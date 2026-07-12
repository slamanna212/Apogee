import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { MantineProvider } from '@mantine/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { primaryMonitor } from '@tauri-apps/api/window';
import { PhysicalPosition, PhysicalSize } from '@tauri-apps/api/dpi';
import { IconApps, IconHistory, IconHome2, IconMinus, IconSettings, IconSquare, IconStar, IconX } from '@tabler/icons-react';
import logoUrl from './assets/logo.svg';
import { theme, cssVariablesResolver } from './theme';
import { useSettingsStore } from './stores/settingsStore';
import { useChannelStore } from './stores/channelStore';
import { usePlayerStore } from './stores/playerStore';
import { useLibraryStore } from './stores/libraryStore';
import { setMediaMetadata } from './lib/mediaSession';
import { TransportBar, type BarMode } from './components/TransportBar';
import { ChannelModal } from './components/ChannelModal';
import { Home } from './pages/Home';
import { Channels } from './pages/Channels';
import { Recent } from './pages/Recent';
import { Favorites } from './pages/Favorites';
import { Settings } from './pages/Settings';

type Page = 'home' | 'channels' | 'recent' | 'favorites' | 'settings';

const NAV_ITEMS: { page: Page; label: string; icon: typeof IconHome2 }[] = [
  { page: 'home', label: 'Home', icon: IconHome2 },
  { page: 'channels', label: 'Channels', icon: IconApps },
  { page: 'recent', label: 'Recent', icon: IconHistory },
  { page: 'favorites', label: 'Favorites', icon: IconStar },
];

const COMPACT_BREAKPOINT = 900;
const CARD_WIDTH = 1180;
const CARD_HEIGHT = 760;
/** Half of the expanded bar's footprint - how far the card's bottom edge sits above the window's bottom, so the bar can overlap it. */
const BAR_OVERLAP = 50;
const EXPANDED_BAR_SIZE = { width: 900, height: 100 };
const COLLAPSED_BAR_SIZE = { width: 340, height: 80 };
const SCREEN_MARGIN = 28;

/**
 * One real OS window throughout. The "browser" is a rounded card absolutely
 * positioned inside it; the transport bar is a second absolutely-positioned
 * layer pinned to the window's bottom edge, overlapping the card by
 * BAR_OVERLAP so it reads as floating on top of it (per the design spec),
 * exactly like a single physical object rather than two windows that merely
 * track each other.
 */
async function applyWindowState(browserOpen: boolean, barMode: BarMode) {
  const win = getCurrentWebviewWindow();
  const monitor = await primaryMonitor();
  const scale = monitor?.scaleFactor ?? 1;

  const target = browserOpen
    ? { width: CARD_WIDTH, height: CARD_HEIGHT + BAR_OVERLAP, resizable: true, alwaysOnTop: false }
    : barMode === 'expanded'
      ? { ...EXPANDED_BAR_SIZE, resizable: false, alwaysOnTop: true }
      : { ...COLLAPSED_BAR_SIZE, resizable: false, alwaysOnTop: true };

  const width = Math.round(target.width * scale);
  const height = Math.round(target.height * scale);

  await win.setResizable(target.resizable);
  await win.setAlwaysOnTop(target.alwaysOnTop);
  await win.setSize(new PhysicalSize(width, height));

  if (monitor) {
    const x = monitor.position.x + (monitor.size.width - width) / 2;
    const y = browserOpen
      ? monitor.position.y + (monitor.size.height - height) / 2
      : monitor.position.y + monitor.size.height - height - Math.round(SCREEN_MARGIN * scale);
    await win.setPosition(new PhysicalPosition(Math.round(x), Math.round(y)));
  }
}

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
  const { settings, loaded: settingsLoaded, load: loadSettings } = useSettingsStore();
  const {
    channels,
    channelMetadata,
    nowPlaying,
    fetchChannels,
    pollNowPlaying,
    fetchChannelMetadata,
  } = useChannelStore();
  const {
    status: playerStatus,
    currentChannel,
    volume,
    errorMessage,
    selectChannel,
    play,
    stop,
    setVolume,
    initEventListener,
  } = usePlayerStore();
  const { loaded: libraryLoaded, load: loadLibrary, recordPlay, favorites, toggleFavorite } = useLibraryStore();

  const [browserOpen, setBrowserOpen] = useState(true);
  const [barMode, setBarMode] = useState<BarMode>('expanded');
  const [page, setPage] = useState<Page>('home');
  const [modalStreamId, setModalStreamId] = useState<number | null>(null);
  const [compact, setCompact] = useState(false);

  useEffect(() => {
    loadSettings();
    loadLibrary();
    initEventListener();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (settingsLoaded) {
      usePlayerStore.setState({ volume: settings.defaultVolume });
    }
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
    if (!settingsLoaded || !settings.stellarApiKey || channels.length === 0) return;
    pollNowPlaying(settings.stellarApiKey);
    const id = setInterval(() => pollNowPlaying(settings.stellarApiKey), settings.pollIntervalSec * 1000);
    return () => clearInterval(id);
  }, [settingsLoaded, settings.stellarApiKey, settings.pollIntervalSec, channels, pollNowPlaying]);

  useEffect(() => {
    if (channels.length > 0) fetchChannelMetadata();
  }, [channels, fetchChannelMetadata]);

  const currentNowPlaying = currentChannel ? nowPlaying.get(currentChannel.stream_id) : undefined;

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
    void applyWindowState(browserOpen, barMode);
  }, [browserOpen, barMode]);

  const windowStateRef = useRef({ browserOpen, barMode });
  useEffect(() => {
    windowStateRef.current = { browserOpen, barMode };
  }, [browserOpen, barMode]);

  useEffect(() => {
    // On Linux, undecorated/transparent windows sometimes get left at the
    // wrong OS size/position after being minimized and restored (a GTK/WM
    // quirk, not something the app's own state tracks). Regaining focus is
    // a reliable signal that the window just came back, so re-assert the
    // size/position for the current mode at that point.
    const win = getCurrentWebviewWindow();
    let unlisten: (() => void) | undefined;
    win
      .onFocusChanged(async ({ payload: focused }) => {
        if (!focused) return;
        if (await win.isMinimized()) return;
        const { browserOpen, barMode } = windowStateRef.current;
        void applyWindowState(browserOpen, barMode);
      })
      .then((fn) => {
        unlisten = fn;
      });
    return () => unlisten?.();
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

  function handleOpenChannel(streamId: number) {
    setModalStreamId(streamId);
  }

  async function handlePlayChannel(streamId: number) {
    const channel = channels.find((c) => c.stream_id === streamId);
    if (!channel) return;
    try {
      await selectChannel(
        channel,
        { baseUrl: settings.baseUrl, username: settings.username, password: settings.password },
        settings.streamExtension,
      );
      if (libraryLoaded) recordPlay(channel.stream_id);
    } catch (err) {
      console.error('playback failed', err);
    }
  }

  const modalChannel = modalStreamId != null ? channels.find((c) => c.stream_id === modalStreamId) : undefined;
  const win = getCurrentWebviewWindow();

  return (
    <div style={{ position: 'relative', width: '100%', height: '100%' }}>
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
            style={{ height: 36, display: 'flex', alignItems: 'center', justifyContent: 'space-between', flex: 'none' }}
          >
            <img src={logoUrl} alt="" width={18} height={18} style={{ marginLeft: 12 }} />
            <div style={{ display: 'flex' }}>
              <TitlebarButton label="Minimize" onClick={() => win.minimize()}>
                <IconMinus size={14} />
              </TitlebarButton>
              <TitlebarButton label="Maximize" onClick={() => win.toggleMaximize()}>
                <IconSquare size={12} />
              </TitlebarButton>
              <TitlebarButton label="Quit" onClick={() => win.close()}>
                <IconX size={14} />
              </TitlebarButton>
            </div>
          </div>

          <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
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
              {page === 'settings' && <Settings />}
            </div>
          </div>
        </div>
      )}

      <div style={{ position: 'absolute', left: '50%', bottom: 0, transform: 'translateX(-50%)', zIndex: 10 }}>
        <TransportBar
          mode={barMode}
          status={playerStatus}
          currentChannel={currentChannel}
          nowPlaying={currentNowPlaying}
          volume={volume}
          onPlus={handlePlus}
          onMinus={handleMinus}
          errorMessage={errorMessage}
          onPlayStop={() => {
            const action = playerStatus === 'playing' || playerStatus === 'loading' ? stop() : play();
            action.catch((err) => console.error('play/stop failed', err));
          }}
          onVolumeChange={setVolume}
        />
      </div>

      {modalChannel && (
        <ChannelModal
          channel={modalChannel}
          metadata={channelMetadata.get(modalChannel.stream_id)}
          apiKey={settings.stellarApiKey}
          isFavorite={favorites.includes(modalChannel.stream_id)}
          onToggleFavorite={() => toggleFavorite(modalChannel.stream_id)}
          onClose={() => setModalStreamId(null)}
        />
      )}
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
      <AppContent />
    </MantineProvider>
  );
}

export default App;
