import { forwardRef, useImperativeHandle, useMemo, useState } from 'react';
import { Button, Group, ScrollArea, Switch, Table, Text } from '@mantine/core';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useSettingsStore } from '../../stores/settingsStore';
import { useChannelStore } from '../../stores/channelStore';
import { listUnmatchedChannels } from '../../lib/channelMatcher';
import type { SettingsPanelProps, SettingsResetHandle } from './types';

export const DiagnosticsPanel = forwardRef<SettingsResetHandle, SettingsPanelProps>(function DiagnosticsPanel(
  { onSaved },
  ref,
) {
  const verboseLogging = useSettingsStore((s) => s.settings.verboseLogging);
  const updateSettings = useSettingsStore((s) => s.update);

  const [logExportStatus, setLogExportStatus] = useState<'idle' | 'saving' | 'ok' | 'error'>('idle');
  const [logExportError, setLogExportError] = useState<string | null>(null);

  const channels = useChannelStore((s) => s.channels);
  const channelMetadata = useChannelStore((s) => s.channelMetadata);
  const metadataStatus = useChannelStore((s) => s.metadataStatus);
  const unmatchedChannels = useMemo(
    () => (metadataStatus === 'loaded' ? listUnmatchedChannels(channels, channelMetadata) : []),
    [channels, channelMetadata, metadataStatus],
  );

  useImperativeHandle(ref, () => ({
    async reset() {
      await updateSettings({ verboseLogging: false });
      await invoke('set_log_level', { verbose: false });
    },
  }));

  function handleVerboseChange(verbose: boolean) {
    void updateSettings({ verboseLogging: verbose });
    void invoke('set_log_level', { verbose });
    onSaved();
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

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
      <Switch
        label="Verbose logging"
        description="Logs playback engine state changes and a periodic bitrate heartbeat - turn on before reproducing a playback issue, then download the log below"
        checked={verboseLogging}
        onChange={(e) => handleVerboseChange(e.currentTarget.checked)}
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
        <Text c="red" size="sm">
          Couldn't save log file: {logExportError}
        </Text>
      )}

      <div>
        <Text fw={600} size="sm">
          Unmatched channels
        </Text>
        <Text size="xs" c="dimmed">
          Channels whose name couldn't be matched to a StellarTunerLog station - they play fine, but show no logo,
          categories or now-playing data. The parsed columns show what the matcher made of the name.
        </Text>
        {metadataStatus !== 'loaded' ? (
          <Text size="sm" c="dimmed" mt={8}>
            {metadataStatus === 'error'
              ? "Couldn't load the StellarTunerLog channel list."
              : 'Waiting for the StellarTunerLog channel list…'}
          </Text>
        ) : unmatchedChannels.length === 0 ? (
          <Text size="sm" c="teal" mt={8}>
            All {channels.length} channels matched
          </Text>
        ) : (
          <ScrollArea.Autosize mah={280} mt={8}>
            <Table striped withTableBorder stickyHeader fz="xs">
              <Table.Thead>
                <Table.Tr>
                  <Table.Th>Provider name</Table.Th>
                  <Table.Th>Parsed name</Table.Th>
                  <Table.Th>Parsed no.</Table.Th>
                  <Table.Th>Stream ID</Table.Th>
                </Table.Tr>
              </Table.Thead>
              <Table.Tbody>
                {unmatchedChannels.map((entry) => (
                  <Table.Tr key={entry.streamId}>
                    <Table.Td>{entry.rawName}</Table.Td>
                    <Table.Td>{entry.parsedName}</Table.Td>
                    <Table.Td>{entry.parsedNumber ?? '—'}</Table.Td>
                    <Table.Td>{entry.streamId}</Table.Td>
                  </Table.Tr>
                ))}
              </Table.Tbody>
            </Table>
          </ScrollArea.Autosize>
        )}
      </div>
    </div>
  );
});
