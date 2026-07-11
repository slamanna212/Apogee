import { useEffect, useState } from 'react';
import { Alert, Button, Group, NumberInput, PasswordInput, Select, Slider, Text, TextInput } from '@mantine/core';
import { useSettingsStore } from '../stores/settingsStore';
import { useLibraryStore, type ThemeMode } from '../stores/libraryStore';
import { getLiveCategories } from '../lib/xtream';
import type { XtreamCategory } from '../types/xtream';

const THEME_OPTIONS: { value: ThemeMode; label: string }[] = [
  { value: 'system', label: 'System default' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
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
