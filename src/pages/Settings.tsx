import { useEffect, useState } from 'react';
import { Alert, Badge, Button, Group, Loader, Paper, PasswordInput, Select, Stack, Switch, Text, TextInput } from '@mantine/core';
import { IconBrandLastfm } from '@tabler/icons-react';
import { getVersion } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useSettingsStore, type UpdateChannel } from '../stores/settingsStore';
import { listAudioDevices, setProperty as mpvSetProperty, type AudioDevice } from '../lib/mpvClient';
import { setWaveformDevice } from '../lib/waveform';
import { useLibraryStore, type ThemeMode } from '../stores/libraryStore';
import { useUpdateStore } from '../stores/updateStore';
import { useAlertsStore } from '../stores/alertsStore';
import { useScrobblingStore } from '../stores/scrobblingStore';
import { getLiveCategories } from '../lib/xtream';
import type { XtreamCategory } from '../types/xtream';
import logoUrl from '../assets/logo.svg';

const THEME_OPTIONS: { value: ThemeMode; label: string }[] = [
  { value: 'system', label: 'System default' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
];

const UPDATE_CHANNEL_OPTIONS: { value: UpdateChannel; label: string }[] = [
  { value: 'stable', label: 'Stable' },
  { value: 'beta', label: 'Beta (pre-releases)' },
];

function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ background: 'var(--app-panel)', border: '1px solid var(--app-border)', borderRadius: 16, padding: 20 }}>
      <Text size="sm" fw={600} c="dimmed" mb={14}>
        {title}
      </Text>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>{children}</div>
    </div>
  );
}

function ProviderSettings({
  name,
  connected,
  username,
  children,
}: {
  name: string;
  connected: boolean;
  username: string | null;
  children: React.ReactNode;
}) {
  return (
    <Paper p="md" radius="md" withBorder style={{ background: 'var(--app-panel)' }}>
      <Stack gap="sm">
        <Group justify="space-between" align="center">
          <Group gap="sm">
            <IconBrandLastfm size={28} color="#d51007" />
            <div>
              <Text fw={600}>{name}</Text>
              {username && <Text size="xs" c="dimmed">Connected as {username}</Text>}
            </div>
          </Group>
          <Badge color={connected ? 'teal' : 'gray'} variant="light">
            {connected ? 'Connected' : 'Not connected'}
          </Badge>
        </Group>
        {children}
      </Stack>
    </Paper>
  );
}

export function Settings() {
  const settings = useSettingsStore((s) => s.settings);
  const settingsLoaded = useSettingsStore((s) => s.loaded);
  const updateSettings = useSettingsStore((s) => s.update);
  const themeMode = useLibraryStore((s) => s.themeMode);
  const setThemeMode = useLibraryStore((s) => s.setThemeMode);
  const notifyOS = useAlertsStore((s) => s.notifyOS);
  const notifyInApp = useAlertsStore((s) => s.notifyInApp);
  const setNotifyOS = useAlertsStore((s) => s.setNotifyOS);
  const setNotifyInApp = useAlertsStore((s) => s.setNotifyInApp);
  const lastFm = useScrobblingStore((s) => s.providers.lastfm);
  const pendingLastFmToken = useScrobblingStore((s) => s.pendingLastFmToken);
  const beginLastFmConnection = useScrobblingStore((s) => s.beginLastFmConnection);
  const finishLastFmConnection = useScrobblingStore((s) => s.finishLastFmConnection);
  const disconnectLastFm = useScrobblingStore((s) => s.disconnectLastFm);

  const [baseUrl, setBaseUrl] = useState(settings.baseUrl);
  const [username, setUsername] = useState(settings.username);
  const [password, setPassword] = useState(settings.password);
  const [categories, setCategories] = useState<XtreamCategory[]>([]);
  const [categoryId, setCategoryId] = useState<string | null>(settings.categoryId);
  const [testStatus, setTestStatus] = useState<'idle' | 'testing' | 'ok' | 'error'>('idle');
  const [testError, setTestError] = useState<string | null>(null);
  const [appVersion, setAppVersion] = useState<string | null>(null);
  const [checkedUpToDate, setCheckedUpToDate] = useState(false);

  const updateStatus = useUpdateStore((s) => s.status);
  const checkForUpdates = useUpdateStore((s) => s.checkForUpdates);

  const [logExportStatus, setLogExportStatus] = useState<'idle' | 'saving' | 'ok' | 'error'>('idle');
  const [logExportError, setLogExportError] = useState<string | null>(null);

  const [audioDevices, setAudioDevices] = useState<AudioDevice[]>([]);
  const [loadingAudioDevices, setLoadingAudioDevices] = useState(false);

  // Enumerating spawns mpv idle, so only do it when the picker is actually
  // opened rather than on every Settings visit. mpv's own "auto" entry is
  // dropped in favor of the explicit "System default" option below.
  async function loadAudioDevices() {
    if (loadingAudioDevices) return;
    setLoadingAudioDevices(true);
    try {
      setAudioDevices((await listAudioDevices()).filter((d) => d.name !== 'auto'));
    } catch {
      // Leave the list as-is; the stored selection (if any) still shows.
    } finally {
      setLoadingAudioDevices(false);
    }
  }

  async function handleAudioDeviceChange(value: string | null) {
    if (!value) {
      await updateSettings({ audioDevice: null });
      mpvSetProperty('audio-device', 'auto').catch(() => {});
      void setWaveformDevice(null, null);
      return;
    }
    const device = audioDevices.find((d) => d.name === value);
    const selection = { name: value, description: device?.description ?? value };
    await updateSettings({ audioDevice: selection });
    mpvSetProperty('audio-device', selection.name).catch(() => {});
    void setWaveformDevice(selection.name, selection.description);
  }

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {});
  }, []);

  async function handleCheckForUpdates() {
    setCheckedUpToDate(false);
    await checkForUpdates(settings.updateChannel);
    if (useUpdateStore.getState().status === 'idle') setCheckedUpToDate(true);
  }

  async function handleDownloadLog() {
    setLogExportStatus('saving');
    setLogExportError(null);
    try {
      const destination = await save({
        defaultPath: `apogee-log-${new Date().toISOString().slice(0, 10)}.log`,
        filters: [{ name: 'Log file', extensions: ['log'] }],
      });
      if (!destination) {
        setLogExportStatus('idle');
        return;
      }
      await invoke('export_log_file', { destination });
      setLogExportStatus('ok');
    } catch (err) {
      setLogExportStatus('error');
      setLogExportError(err instanceof Error ? err.message : String(err));
    }
  }

  useEffect(() => {
    if (!settingsLoaded) return;
    setBaseUrl(settings.baseUrl);
    setUsername(settings.username);
    setPassword(settings.password);
    setCategoryId(settings.categoryId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settingsLoaded]);

  async function handleTestConnection() {
    setTestStatus('testing');
    setTestError(null);
    try {
      const cats = await getLiveCategories({ baseUrl, username, password });
      setCategories(cats);
      setTestStatus('ok');
    } catch (err) {
      setTestStatus('error');
      setTestError(err instanceof Error ? err.message : String(err));
    }
  }

  function handleSave() {
    const category = categories.find((c) => c.category_id === categoryId);
    updateSettings({
      baseUrl,
      username,
      password,
      categoryId,
      categoryName: category?.category_name ?? settings.categoryName,
    });
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
      <div style={{ font: '700 24px "Space Grotesk", sans-serif', marginBottom: 24, width: '100%', maxWidth: 600 }}>Settings</div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 16, width: '100%', maxWidth: 600 }}>
        <Card title="About">
          <Group gap={14}>
            <img src={logoUrl} alt="Apogee" width={40} height={40} />
            <div>
              <Text fw={700} size="md" style={{ fontFamily: '"Space Grotesk", sans-serif' }}>
                Apogee
              </Text>
              <Text size="sm" c="dimmed">
                {appVersion ? `Version ${appVersion}` : 'Xtream Codes radio player'}
              </Text>
            </div>
          </Group>
        </Card>

        <Card title="Xtream connection">
          <TextInput label="Xtream base URL" placeholder="http://host:port" value={baseUrl} onChange={(e) => setBaseUrl(e.currentTarget.value)} />
          <Group grow>
            <TextInput label="Username" value={username} onChange={(e) => setUsername(e.currentTarget.value)} />
            <PasswordInput label="Password" value={password} onChange={(e) => setPassword(e.currentTarget.value)} />
          </Group>
          <Group align="center">
            <Button onClick={handleTestConnection} loading={testStatus === 'testing'}>
              Test connection
            </Button>
            {testStatus === 'ok' && (
              <Text c="teal" size="sm">
                Connected — {categories.length} categories found
              </Text>
            )}
          </Group>
          {testStatus === 'error' && (
            <Alert color="red" title="Connection failed">
              {testError}
            </Alert>
          )}
          <Select
            label="Channel group"
            placeholder="Run Test connection to load groups"
            data={categories.map((c) => ({ value: c.category_id, label: c.category_name }))}
            value={categoryId}
            onChange={setCategoryId}
            disabled={categories.length === 0}
            searchable
          />
        </Card>

        <Card title="Appearance">
          <Select
            label="Theme"
            data={THEME_OPTIONS}
            value={themeMode}
            onChange={(v) => setThemeMode((v as ThemeMode) ?? 'system')}
            allowDeselect={false}
          />
          <Switch
            label="Keep window on top when using the mini player"
            description="Applies to the expanded and collapsed mini player only, not the full window"
            checked={settings.keepMiniWindowOnTop}
            onChange={(e) => updateSettings({ keepMiniWindowOnTop: e.currentTarget.checked })}
          />
        </Card>

        <Card title="Audio">
          <Select
            label="Output device"
            description="Where audio plays, and the source the visualizer listens to"
            placeholder="System default"
            data={[
              { value: '', label: 'System default' },
              // Surface the saved device even before the list has loaded so it
              // shows its label instead of a bare id on first open.
              ...(settings.audioDevice && !audioDevices.some((d) => d.name === settings.audioDevice!.name)
                ? [{ value: settings.audioDevice.name, label: settings.audioDevice.description || settings.audioDevice.name }]
                : []),
              ...audioDevices.map((d) => ({ value: d.name, label: d.description || d.name })),
            ]}
            value={settings.audioDevice?.name ?? ''}
            onChange={handleAudioDeviceChange}
            onDropdownOpen={() => { if (audioDevices.length === 0) void loadAudioDevices(); }}
            rightSection={loadingAudioDevices ? <Loader size="xs" /> : undefined}
            allowDeselect={false}
            searchable
          />
        </Card>

        <Card title="Discord">
          <Switch
            label="Show now playing on Discord"
            description="Displays the current channel and track (when matched) as your Discord status via Rich Presence"
            checked={settings.discordRpcEnabled}
            onChange={(e) => updateSettings({ discordRpcEnabled: e.currentTarget.checked })}
          />
        </Card>

        <Card title="Scrobbling">
          <Text size="sm" c="dimmed">
            Share the songs you listen to with connected music services. Only items identified as songs are submitted.
          </Text>
          <ProviderSettings name="Last.fm" connected={lastFm.connected} username={lastFm.username}>
            {!lastFm.available && lastFm.status !== 'loading' ? (
              <Alert color="yellow" title="Unavailable in this build">
                Last.fm credentials were not configured when this version of Apogee was built.
              </Alert>
            ) : lastFm.connected ? (
              <>
                <Switch
                  label="Scrobble to Last.fm"
                  description="Shows now playing immediately and scrobbles after 25 seconds of listening"
                  checked={settings.scrobbling.lastfm.enabled}
                  onChange={(event) => updateSettings({
                    scrobbling: { lastfm: { enabled: event.currentTarget.checked } },
                  })}
                />
                <Group justify="flex-end">
                  <Button
                    variant="subtle"
                    color="red"
                    onClick={() => {
                      void updateSettings({ scrobbling: { lastfm: { enabled: false } } });
                      void disconnectLastFm();
                    }}
                  >
                    Disconnect
                  </Button>
                </Group>
              </>
            ) : pendingLastFmToken ? (
              <>
                <Text size="sm" c="dimmed">
                  Approve Apogee in the browser, then finish the connection here.
                </Text>
                <Group>
                  <Button onClick={() => void finishLastFmConnection()} loading={lastFm.status === 'loading'}>
                    Finish connection
                  </Button>
                  <Button variant="default" onClick={() => void beginLastFmConnection()} disabled={lastFm.status === 'loading'}>
                    Restart authorization
                  </Button>
                </Group>
              </>
            ) : (
              <Button onClick={() => void beginLastFmConnection()} loading={lastFm.status === 'loading'} style={{ alignSelf: 'flex-start' }}>
                Connect Last.fm
              </Button>
            )}
            {lastFm.error && <Alert color="red">{lastFm.error}</Alert>}
          </ProviderSettings>
        </Card>

        <Card title="Alerts">
          <Switch
            label="In-app notifications"
            description="Show a toast inside Apogee when a followed track or artist starts playing"
            checked={notifyInApp}
            onChange={(e) => setNotifyInApp(e.currentTarget.checked)}
          />
          <Switch
            label="OS notifications"
            description="Show a system notification, even when Apogee is minimized or in the mini player"
            checked={notifyOS}
            onChange={(e) => setNotifyOS(e.currentTarget.checked)}
          />
        </Card>

        <Card title="Updates">
          <Select
            label="Update channel"
            data={UPDATE_CHANNEL_OPTIONS}
            value={settings.updateChannel}
            onChange={(v) => updateSettings({ updateChannel: (v as UpdateChannel) ?? 'stable' })}
            allowDeselect={false}
          />
          <Group align="center">
            <Button onClick={handleCheckForUpdates} loading={updateStatus === 'checking'}>
              Check for Updates
            </Button>
            {checkedUpToDate && (
              <Text c="teal" size="sm">
                You're up to date
              </Text>
            )}
          </Group>
        </Card>

        <Card title="Diagnostics">
          <Switch
            label="Verbose logging"
            description="Logs every mpv event, command, and a periodic playback heartbeat - turn on before reproducing a playback issue, then download the log below"
            checked={settings.verboseLogging}
            onChange={(e) => {
              const verbose = e.currentTarget.checked;
              updateSettings({ verboseLogging: verbose });
              invoke('set_log_level', { verbose });
            }}
          />
          <Group align="center">
            <Button onClick={handleDownloadLog} loading={logExportStatus === 'saving'}>
              Download log file
            </Button>
            {logExportStatus === 'ok' && (
              <Text c="teal" size="sm">
                Saved
              </Text>
            )}
          </Group>
          {logExportStatus === 'error' && (
            <Alert color="red" title="Couldn't save log file">
              {logExportError}
            </Alert>
          )}
        </Card>

        <Group justify="flex-end">
          <Button onClick={handleSave} disabled={!categoryId}>
            Save
          </Button>
        </Group>
      </div>
    </div>
  );
}
