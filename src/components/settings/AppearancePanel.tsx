import { forwardRef, useImperativeHandle } from 'react';
import { Select, Switch, Text } from '@mantine/core';
import { OptionRow } from './OptionRow';
import { useLibraryStore, type ThemeMode } from '../../stores/libraryStore';
import { useSettingsStore, type StartupPage, type HomePageSize } from '../../stores/settingsStore';
import type { SettingsPanelProps, SettingsResetHandle } from './types';

const THEME_OPTIONS: { value: ThemeMode; label: string }[] = [
  { value: 'system', label: 'System' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
];

export const AppearancePanel = forwardRef<SettingsResetHandle, SettingsPanelProps>(function AppearancePanel(
  { onSaved },
  ref,
) {
  const themeMode = useLibraryStore((s) => s.themeMode);
  const setThemeMode = useLibraryStore((s) => s.setThemeMode);
  const keepMiniWindowOnTop = useSettingsStore((s) => s.settings.keepMiniWindowOnTop);
  const startupPage = useSettingsStore((s) => s.settings.startupPage);
  const homePageSize = useSettingsStore((s) => s.settings.homePageSize);
  const updateSettings = useSettingsStore((s) => s.update);

  useImperativeHandle(ref, () => ({
    async reset() {
      await Promise.all([setThemeMode('system'), updateSettings({ keepMiniWindowOnTop: true, startupPage: 'home', homePageSize: 'standard' })]);
    },
  }));

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 30 }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <label style={{ font: "600 13px 'Sora', sans-serif" }}>Theme</label>
        <OptionRow
          options={THEME_OPTIONS}
          value={themeMode}
          onChange={(mode) => {
            void setThemeMode(mode);
            onSaved();
          }}
        />
      </div>
      <Select
        label="Startup page"
        description="The page shown when Apogee opens"
        data={[
          { value: 'home', label: 'Home' },
          { value: 'channels', label: 'Channels' },
          { value: 'favorites', label: 'Favorites' },
          { value: 'recent', label: 'Recent' },
          { value: 'alerts', label: 'Alerts' },
        ]}
        value={startupPage}
        allowDeselect={false}
        onChange={(value) => {
          if (!value) return;
          void updateSettings({ startupPage: value as StartupPage });
          onSaved();
        }}
      />
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <label style={{ font: "600 13px 'Sora', sans-serif" }}>Home-page size</label>
        <Text size="xs" c="dimmed">How many recommendations and recent channels appear on Home</Text>
        <OptionRow<HomePageSize>
          options={[
            { value: 'small', label: 'Small' },
            { value: 'standard', label: 'Standard' },
            { value: 'expanded', label: 'Expanded' },
          ]}
          value={homePageSize}
          onChange={(value) => {
            void updateSettings({ homePageSize: value });
            onSaved();
          }}
        />
      </div>
      <div style={{ height: 1, background: 'var(--app-border)' }} />
      <Switch
        label="Keep window on top when using the mini player"
        description="Applies to the expanded and collapsed mini player only, not the full window"
        checked={keepMiniWindowOnTop}
        onChange={(e) => {
          void updateSettings({ keepMiniWindowOnTop: e.currentTarget.checked });
          onSaved();
        }}
      />
    </div>
  );
});
