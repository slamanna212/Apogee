import { forwardRef, useImperativeHandle } from 'react';
import { Switch } from '@mantine/core';
import { OptionRow } from './OptionRow';
import { useLibraryStore, type ThemeMode } from '../../stores/libraryStore';
import { useSettingsStore } from '../../stores/settingsStore';
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
  const updateSettings = useSettingsStore((s) => s.update);

  useImperativeHandle(ref, () => ({
    async reset() {
      await Promise.all([setThemeMode('system'), updateSettings({ keepMiniWindowOnTop: true })]);
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
