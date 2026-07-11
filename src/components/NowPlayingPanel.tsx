import { Avatar, Badge, Group, Stack, Text } from '@mantine/core';
import { usePlayerStore } from '../stores/playerStore';
import { TransportControls } from './TransportControls';

const STATUS_LABEL: Record<string, string> = {
  idle: 'Select a channel to start listening',
  loading: 'Connecting…',
  playing: 'Playing',
  paused: 'Paused',
  error: 'Playback error',
};

export function NowPlayingPanel() {
  const { currentChannel, status, bitrateKbps } = usePlayerStore();

  if (!currentChannel) {
    return <Text c="dimmed">{STATUS_LABEL.idle}</Text>;
  }

  return (
    <Stack>
      <Group>
        <Avatar src={currentChannel.stream_icon} size="lg" radius="sm">
          {currentChannel.name.charAt(0)}
        </Avatar>
        <div>
          <Text fw={600}>{currentChannel.name}</Text>
          <Group gap="xs">
            <Text size="sm" c="dimmed">
              {STATUS_LABEL[status]}
            </Text>
            {bitrateKbps != null && <Badge variant="light">{bitrateKbps} kbps</Badge>}
          </Group>
        </div>
      </Group>
      <TransportControls />
    </Stack>
  );
}
