import { useEffect, useState } from 'react';
import { Button, Group, Text } from '@mantine/core';
import { IconBug, IconExternalLink } from '@tabler/icons-react';
import { getVersion } from '@tauri-apps/api/app';
import { openUrl } from '@tauri-apps/plugin-opener';
import logoUrl from '../../assets/logo.svg';

const REPO = 'slamanna212/Apogee';

/** No editable settings here, so no reset support and no onSaved prop (see
 *  hideReset on the 'about' category in ./categories). */
export function AboutPanel() {
  const [appVersion, setAppVersion] = useState<string | null>(null);

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {});
  }, []);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <Group gap={14}>
        <img src={logoUrl} alt="Apogee" width={44} height={44} />
        <div>
          <Text fw={700} size="lg" style={{ fontFamily: '"Space Grotesk", sans-serif' }}>
            Apogee
          </Text>
          <Text size="sm" c="dimmed">
            {appVersion ? `Version ${appVersion}` : 'Xtream Codes radio player'}
          </Text>
        </div>
      </Group>
      <Group gap={10}>
        <Button
          variant="default"
          leftSection={<IconExternalLink size={15} />}
          onClick={() => void openUrl(`https://github.com/${REPO}/releases`)}
        >
          Release notes
        </Button>
        <Button
          variant="default"
          leftSection={<IconBug size={15} />}
          onClick={() => void openUrl(`https://github.com/${REPO}/issues/new/choose`)}
        >
          Report a bug
        </Button>
      </Group>
    </div>
  );
}
