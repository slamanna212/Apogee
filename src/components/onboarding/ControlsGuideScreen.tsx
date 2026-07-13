import { Button, Group, Text } from '@mantine/core';
import { IconKeyboard, IconLayoutGrid, IconStar } from '@tabler/icons-react';
import type { Icon } from '@tabler/icons-react';
import type { ReactNode } from 'react';

interface ControlRow {
  icon: Icon;
  title: string;
  description: ReactNode;
}

const glyphStyle = { font: '700 15px "Space Grotesk", sans-serif', color: 'var(--app-text)' };

const CONTROL_ROWS: ControlRow[] = [
  {
    icon: IconLayoutGrid,
    title: 'Three window modes',
    description: (
      <>
        Use the <span style={glyphStyle}>+</span> and <span style={glyphStyle}>&minus;</span> buttons on the player
        bar to switch between the full app, a mini player, and a compact floating bar.
      </>
    ),
  },
  {
    icon: IconKeyboard,
    title: 'Media key support',
    description: 'Play/pause and stop from your keyboard or OS media controls work too.',
  },
  {
    icon: IconStar,
    title: 'Favorite your channels',
    description: 'Star a channel from its card or detail view to find it quickly under Favorites.',
  },
];

export function ControlsGuideScreen({ onBack, onFinish }: { onBack: () => void; onFinish: () => void }) {
  return (
    <div style={{ flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 16, width: '100%', maxWidth: 560 }}>
      <div style={{ textAlign: 'center' }}>
        <div style={{ font: '700 22px "Space Grotesk", sans-serif', marginBottom: 6 }}>Getting around Apogee</div>
        <Text size="sm" c="dimmed">
          A few things worth knowing before you dive in.
        </Text>
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 16, width: '100%' }}>
        {CONTROL_ROWS.map(({ icon: Icon, title, description }) => (
          <div
            key={title}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 20,
              background: 'var(--app-panel)',
              border: '1px solid var(--app-border)',
              borderRadius: 20,
              padding: '22px 26px',
            }}
          >
            <div
              style={{
                width: 56,
                height: 56,
                borderRadius: 16,
                background: 'var(--app-accent-soft)',
                color: 'var(--app-accent)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                flex: 'none',
              }}
            >
              <Icon size={28} />
            </div>
            <div>
              <div style={{ font: '600 16px "Sora", sans-serif', color: 'var(--app-text)', marginBottom: 3 }}>{title}</div>
              <div style={{ font: '400 14px "Sora", sans-serif', color: 'var(--app-dim)', lineHeight: 1.5 }}>{description}</div>
            </div>
          </div>
        ))}
      </div>

      <Group justify="space-between" w="100%" mt={4}>
        <Button variant="subtle" onClick={onBack}>
          Back
        </Button>
        <Button onClick={onFinish}>Start listening</Button>
      </Group>
    </div>
  );
}
