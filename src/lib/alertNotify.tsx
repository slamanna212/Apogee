import { Button, Group, Text } from '@mantine/core';
import { notifications } from '@mantine/notifications';
import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification';
import type { AlertEntry } from '../types/alerts';
import type { StellarStation } from '../types/stellarTunerLog';

/** actionTypeId registered once at app startup via registerActionTypes - see App.tsx. */
export const ALERT_ACTION_TYPE_ID = 'channel-alert';
export const ALERT_GO_TO_ACTION_ID = 'go-to-channel';

/** Requests OS notification permission if not already decided. Safe to call repeatedly. */
export async function ensureOSPermission(): Promise<boolean> {
  if (await isPermissionGranted()) return true;
  return (await requestPermission()) === 'granted';
}

export async function fireAlert(
  entry: AlertEntry,
  station: StellarStation,
  streamId: number,
  channelName: string,
  notifyOS: boolean,
  notifyInApp: boolean,
  onGoToChannel?: (streamId: number) => void,
) {
  const title = entry.type === 'artist' ? `${station.artist} — now on ${channelName}` : `${station.title} is playing`;
  const body = entry.type === 'artist' ? station.title : `${station.artist} • ${channelName}`;

  if (notifyInApp) {
    const id = `alert-${streamId}-${Date.now()}`;
    notifications.show({
      id,
      title,
      autoClose: 8000,
      message: (
        <Group gap={8} wrap="nowrap" justify="space-between" align="center">
          <Text size="sm">{body}</Text>
          {onGoToChannel && (
            <Button
              size="xs"
              variant="light"
              onClick={() => {
                onGoToChannel(streamId);
                notifications.hide(id);
              }}
            >
              Tune
            </Button>
          )}
        </Group>
      ),
    });
  }
  if (notifyOS && (await ensureOSPermission())) {
    sendNotification({
      title,
      body,
      actionTypeId: ALERT_ACTION_TYPE_ID,
      extra: { streamId },
    });
  }
}
