import { forwardRef, useImperativeHandle } from 'react';
import { Alert, Badge, Button, Group, Paper, Stack, Switch, Text } from '@mantine/core';
import { IconBrandLastfm } from '@tabler/icons-react';
import { useSettingsStore } from '../../stores/settingsStore';
import { useScrobblingStore } from '../../stores/scrobblingStore';
import type { SettingsPanelProps, SettingsResetHandle } from './types';

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

export const ScrobblingPanel = forwardRef<SettingsResetHandle, SettingsPanelProps>(function ScrobblingPanel(
  { onSaved },
  ref,
) {
  const updateSettings = useSettingsStore((s) => s.update);
  const lastfmEnabled = useSettingsStore((s) => s.settings.scrobbling.lastfm.enabled);
  const lastFm = useScrobblingStore((s) => s.providers.lastfm);
  const pendingLastFmToken = useScrobblingStore((s) => s.pendingLastFmToken);
  const beginLastFmConnection = useScrobblingStore((s) => s.beginLastFmConnection);
  const finishLastFmConnection = useScrobblingStore((s) => s.finishLastFmConnection);
  const disconnectLastFm = useScrobblingStore((s) => s.disconnectLastFm);

  useImperativeHandle(ref, () => ({
    // Resets the "scrobble" toggle to its default (off); it does not disconnect
    // the Last.fm account itself, since that's a connection, not a setting.
    async reset() {
      await updateSettings({ scrobbling: { lastfm: { enabled: false } } });
    },
  }));

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <Text size="sm" c="dimmed" maw={520}>
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
              checked={lastfmEnabled}
              onChange={(event) => {
                void updateSettings({ scrobbling: { lastfm: { enabled: event.currentTarget.checked } } });
                onSaved();
              }}
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
    </div>
  );
});
