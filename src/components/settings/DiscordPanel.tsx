import { forwardRef, useImperativeHandle } from 'react';
import { Switch } from '@mantine/core';
import { useSettingsStore } from '../../stores/settingsStore';
import type { SettingsPanelProps, SettingsResetHandle } from './types';

export const DiscordPanel = forwardRef<SettingsResetHandle, SettingsPanelProps>(function DiscordPanel({ onSaved }, ref) {
  const discordRpcEnabled = useSettingsStore((s) => s.settings.discordRpcEnabled);
  const updateSettings = useSettingsStore((s) => s.update);

  useImperativeHandle(ref, () => ({
    async reset() {
      await updateSettings({ discordRpcEnabled: false });
    },
  }));

  return (
    <Switch
      label="Show now playing on Discord"
      description="Displays the current channel and track (when matched) as your Discord status via Rich Presence"
      checked={discordRpcEnabled}
      onChange={(e) => {
        void updateSettings({ discordRpcEnabled: e.currentTarget.checked });
        onSaved();
      }}
    />
  );
});
