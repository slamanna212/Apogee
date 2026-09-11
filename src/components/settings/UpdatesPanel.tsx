import { forwardRef, useImperativeHandle, useState } from 'react';
import { Button, Group, Text } from '@mantine/core';
import { OptionRow } from './OptionRow';
import { useSettingsStore, type UpdateChannel } from '../../stores/settingsStore';
import { useUpdateStore } from '../../stores/updateStore';
import type { SettingsPanelProps, SettingsResetHandle } from './types';

const UPDATE_CHANNEL_OPTIONS: { value: UpdateChannel; label: string }[] = [
  { value: 'stable', label: 'Stable' },
  { value: 'beta', label: 'Beta (pre-releases)' },
];

export const UpdatesPanel = forwardRef<SettingsResetHandle, SettingsPanelProps>(function UpdatesPanel({ onSaved }, ref) {
  const updateChannel = useSettingsStore((s) => s.settings.updateChannel);
  const updateSettings = useSettingsStore((s) => s.update);
  const updateStatus = useUpdateStore((s) => s.status);
  const checkForUpdates = useUpdateStore((s) => s.checkForUpdates);
  const [checkedUpToDate, setCheckedUpToDate] = useState(false);

  useImperativeHandle(ref, () => ({
    async reset() {
      await updateSettings({ updateChannel: 'stable' });
    },
  }));

  async function handleCheckForUpdates() {
    setCheckedUpToDate(false);
    await checkForUpdates(updateChannel);
    if (useUpdateStore.getState().status === 'idle') setCheckedUpToDate(true);
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <label style={{ font: "600 13px 'Sora', sans-serif" }}>Update channel</label>
        <OptionRow
          options={UPDATE_CHANNEL_OPTIONS}
          value={updateChannel}
          onChange={(channel) => {
            void updateSettings({ updateChannel: channel });
            onSaved();
          }}
        />
      </div>
      <Group align="center">
        <Button onClick={handleCheckForUpdates} loading={updateStatus === 'checking'}>
          Check for updates
        </Button>
        {checkedUpToDate && (
          <Text c="teal" size="sm">
            You're up to date
          </Text>
        )}
      </Group>
    </div>
  );
});
