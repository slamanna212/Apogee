import { forwardRef, useImperativeHandle } from 'react';
import { Switch } from '@mantine/core';
import { useAlertsStore } from '../../stores/alertsStore';
import type { SettingsPanelProps, SettingsResetHandle } from './types';

export const AlertsPanel = forwardRef<SettingsResetHandle, SettingsPanelProps>(function AlertsPanel({ onSaved }, ref) {
  const notifyOS = useAlertsStore((s) => s.notifyOS);
  const notifyInApp = useAlertsStore((s) => s.notifyInApp);
  const setNotifyOS = useAlertsStore((s) => s.setNotifyOS);
  const setNotifyInApp = useAlertsStore((s) => s.setNotifyInApp);

  useImperativeHandle(ref, () => ({
    async reset() {
      await Promise.all([setNotifyOS(true), setNotifyInApp(true)]);
    },
  }));

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
      <Switch
        label="In-app notifications"
        description="Show a toast inside Apogee when a followed track or artist starts playing"
        checked={notifyInApp}
        onChange={(e) => {
          void setNotifyInApp(e.currentTarget.checked);
          onSaved();
        }}
      />
      <Switch
        label="OS notifications"
        description="Show a system notification, even when Apogee is minimized or in the mini player"
        checked={notifyOS}
        onChange={(e) => {
          void setNotifyOS(e.currentTarget.checked);
          onSaved();
        }}
      />
    </div>
  );
});
