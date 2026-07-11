import { Avatar, Badge, Group, Stack, Text } from '@mantine/core';
import { usePlayerStore } from '../stores/playerStore';
import type { StellarStation } from '../types/stellarTunerLog';
import { TransportControls } from './TransportControls';

const STATUS_LABEL: Record<string, string> = {
  idle: 'Select a channel to start listening',
  loading: 'Connecting…',
  playing: 'Playing',
  stopped: 'Stopped',
  error: 'Playback error',
};

interface NowPlayingPanelProps {
  nowPlayingEntry?: StellarStation;
}

export function NowPlayingPanel({ nowPlayingEntry }: NowPlayingPanelProps) {
  const { currentChannel, status, bitrateKbps } = usePlayerStore();

  if (!currentChannel) {
    return <Text c="dimmed">{STATUS_LABEL.idle}</Text>;
  }

  const artworkUrl = nowPlayingEntry?.artwork_url || currentChannel.stream_icon;

  return (
    <Stack>
      <Group align="flex-start">
        <Avatar src={artworkUrl} size={96} radius="sm">
          {currentChannel.name.charAt(0)}
        </Avatar>
        <div>
          <Text size="sm" c="dimmed">
            {currentChannel.name}
          </Text>
          {nowPlayingEntry ? (
            <>
              <Text fw={700} size="lg">
                {nowPlayingEntry.title}
              </Text>
              <Text size="sm">{nowPlayingEntry.artist}</Text>
              {nowPlayingEntry.album && (
                <Text size="xs" c="dimmed">
                  {nowPlayingEntry.album}
                </Text>
              )}
            </>
          ) : (
            <Text fw={600}>{currentChannel.name}</Text>
          )}
          <Group gap="xs" mt={4}>
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
