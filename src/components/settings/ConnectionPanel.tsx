import { useEffect, useRef, useState } from 'react';
import { Alert, Button, Group, PasswordInput, Text, TextInput } from '@mantine/core';
import { useSettingsStore } from '../../stores/settingsStore';
import { getLiveCategories } from '../../lib/xtream';
import type { XtreamCategory } from '../../types/xtream';
import { ChannelGroupSelector } from '../ChannelGroupSelector';
import type { SettingsPanelProps } from './types';

const CONNECTION_DEBOUNCE_MS = 500;

/** No reset support: Connection hides the "Reset to defaults" button (see
 *  categories.ts) since it would wipe live credentials with no undo, so this
 *  panel doesn't need to expose a SettingsResetHandle. */
export function ConnectionPanel({ onSaved }: SettingsPanelProps) {
  const settings = useSettingsStore((s) => s.settings);
  const settingsLoaded = useSettingsStore((s) => s.loaded);
  const updateSettings = useSettingsStore((s) => s.update);

  const [baseUrl, setBaseUrl] = useState(settings.baseUrl);
  const [username, setUsername] = useState(settings.username);
  const [password, setPassword] = useState(settings.password);
  const [categoryIds, setCategoryIds] = useState<string[]>(settings.categoryIds);
  const [categories, setCategories] = useState<XtreamCategory[]>([]);
  const [testStatus, setTestStatus] = useState<'idle' | 'testing' | 'ok' | 'error'>('idle');
  const [testError, setTestError] = useState<string | null>(null);

  // Debounced autosave for the three connection fields: typing updates local state
  // immediately for a responsive input, and the actual write (through the settings
  // store, which also routes the password to the OS keyring) fires 500ms after the
  // user stops typing in any of them, batched into a single update() call.
  const pendingRef = useRef<{ baseUrl: string; username: string; password: string } | null>(null);
  const debounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => {
    if (debounceTimer.current) clearTimeout(debounceTimer.current);
  }, []);

  function scheduleConnectionSave(next: { baseUrl: string; username: string; password: string }) {
    pendingRef.current = next;
    if (debounceTimer.current) clearTimeout(debounceTimer.current);
    debounceTimer.current = setTimeout(() => {
      debounceTimer.current = null;
      const pending = pendingRef.current;
      pendingRef.current = null;
      if (pending) void updateSettings(pending).then(onSaved);
    }, CONNECTION_DEBOUNCE_MS);
  }

  function editBaseUrl(value: string) {
    setBaseUrl(value);
    scheduleConnectionSave({ baseUrl: value, username, password });
  }
  function editUsername(value: string) {
    setUsername(value);
    scheduleConnectionSave({ baseUrl, username: value, password });
  }
  function editPassword(value: string) {
    setPassword(value);
    scheduleConnectionSave({ baseUrl, username, password: value });
  }

  useEffect(() => {
    if (!settingsLoaded) return;
    setBaseUrl(settings.baseUrl);
    setUsername(settings.username);
    setPassword(settings.password);
    setCategoryIds(settings.categoryIds);
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

  // Persists the moment the selection changes, rather than staging it for a Save
  // button - but never down to zero groups: there's no "no channels configured"
  // empty state built for that, so the last selected group can't be deselected.
  function handleGroupsChange(nextIds: string[]) {
    if (nextIds.length === 0) return;
    setCategoryIds(nextIds);
    const names = categories.filter((c) => nextIds.includes(c.category_id)).map((c) => c.category_name);
    void updateSettings({
      categoryIds: nextIds,
      categoryNames: names.length > 0 ? names : settings.categoryNames,
    }).then(onSaved);
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
      <TextInput
        label="Xtream base URL"
        placeholder="http://host:port"
        value={baseUrl}
        onChange={(e) => editBaseUrl(e.currentTarget.value)}
      />
      <Group grow>
        <TextInput label="Username" value={username} onChange={(e) => editUsername(e.currentTarget.value)} />
        <PasswordInput label="Password" value={password} onChange={(e) => editPassword(e.currentTarget.value)} />
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
      <ChannelGroupSelector categories={categories} value={categoryIds} onChange={handleGroupsChange} />
    </div>
  );
}
