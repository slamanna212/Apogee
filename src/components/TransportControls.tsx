import { ActionIcon, Group, Slider, Text } from '@mantine/core';
import { usePlayerStore } from '../stores/playerStore';

export function TransportControls() {
  const { status, volume, togglePause, setVolume, currentChannel } = usePlayerStore();
  const disabled = !currentChannel;

  return (
    <Group>
      <ActionIcon size="lg" disabled={disabled} onClick={() => togglePause()}>
        {status === 'playing' ? '⏸' : '▶'}
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
