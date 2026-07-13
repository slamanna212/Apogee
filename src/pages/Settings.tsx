import { useEffect, useState } from 'react';
import { Alert, Button, Group, NumberInput, PasswordInput, Select, Slider, Switch, Text, TextInput } from '@mantine/core';
import { getVersion } from '@tauri-apps/api/app';
import { useSettingsStore, type UpdateChannel } from '../stores/settingsStore';
import { useLibraryStore, type ThemeMode } from '../stores/libraryStore';
import { useUpdateStore } from '../stores/updateStore';
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

export function Settings() {
  const settings = useSettingsStore((s) => s.settings);
  const settingsLoaded = useSettingsStore((s) => s.loaded);
  const updateSettings = useSettingsStore((s) => s.update);
  const themeMode = useLibraryStore((s) => s.themeMode);
  const setThemeMode = useLibraryStore((s) => s.setThemeMode);

  const [baseUrl, setBaseUrl] = useState(settings.baseUrl);
  const [username, setUsername] = useState(settings.username);
  const [password, setPassword] = useState(settings.password);
  const [streamExtension, setStreamExtension] = useState(settings.streamExtension);
  const [stellarApiKey, setStellarApiKey] = useState(settings.stellarApiKey);
  const [pollIntervalSec, setPollIntervalSec] = useState(settings.pollIntervalSec);
  const [defaultVolume, setDefaultVolume] = useState(settings.defaultVolume);
  const [categories, setCategories] = useState<XtreamCategory[]>([]);
  const [categoryId, setCategoryId] = useState<string | null>(settings.categoryId);
  const [testStatus, setTestStatus] = useState<'idle' | 'testing' | 'ok' | 'error'>('idle');
  const [testError, setTestError] = useState<string | null>(null);
  const [appVersion, setAppVersion] = useState<string | null>(null);
  const [checkedUpToDate, setCheckedUpToDate] = useState(false);

  const updateStatus = useUpdateStore((s) => s.status);
  const checkForUpdates = useUpdateStore((s) => s.checkForUpdates);

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {});
  }, []);

  async function handleCheckForUpdates() {
    setCheckedUpToDate(false);
    await checkForUpdates(settings.updateChannel);
    if (useUpdateStore.getState().status === 'idle') setCheckedUpToDate(true);
  }

  useEffect(() => {
    if (!settingsLoaded) return;
    setBaseUrl(settings.baseUrl);
    setUsername(settings.username);
    setPassword(settings.password);
    setStreamExtension(settings.streamExtension);
    setStellarApiKey(settings.stellarApiKey);
    setPollIntervalSec(settings.pollIntervalSec);
    setDefaultVolume(settings.defaultVolume);
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
      streamExtension,
      stellarApiKey,
      pollIntervalSec,
      defaultVolume,
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

        <Card title="Playback">
          <Select label="Stream extension" data={['.ts', '.m3u8']} value={streamExtension} onChange={(v) => setStreamExtension(v ?? '.ts')} />
          <div>
            <Text size="sm" fw={500} mb={4}>
              Default volume
            </Text>
            <Slider value={defaultVolume} onChange={setDefaultVolume} min={0} max={100} label={(v) => `${v}%`} />
          </div>
        </Card>

        <Card title="StellarTunerLog API">
          <TextInput label="API key" value={stellarApiKey} onChange={(e) => setStellarApiKey(e.currentTarget.value)} />
          <NumberInput
            label="Poll interval (seconds)"
            min={5}
            max={300}
            value={pollIntervalSec}
            onChange={(v) => setPollIntervalSec(typeof v === 'number' ? v : 25)}
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

        <Group justify="flex-end">
          <Button onClick={handleSave} disabled={!categoryId}>
            Save
          </Button>
        </Group>
      </div>
    </div>
  );
}
