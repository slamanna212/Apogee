import { ActionIcon, Group, Loader, Slider, Text } from '@mantine/core';
import { usePlayerStore } from '../stores/playerStore';

export function TransportControls() {
  const { status, volume, play, stop, setVolume, currentChannel } = usePlayerStore();
  const disabled = !currentChannel;
  const isConnected = status === 'playing' || status === 'loading';

  return (
    <Group>
      <ActionIcon
        size="lg"
        disabled={disabled}
        onClick={() => (isConnected ? stop() : play())}
        aria-label={isConnected ? 'Stop' : 'Play'}
      >
        {status === 'loading' ? <Loader size={16} color="white" /> : isConnected ? '⏹' : '▶'}
      </ActionIcon>
      <Text size="sm" c="dimmed" w={70}>
        Volume
      </Text>
      <Slider
        w={200}
        value={volume}
        onChange={setVolume}
        min={0}
        max={100}
        disabled={disabled}
        label={(v) => `${v}%`}
      />
    </Group>
  );
}
