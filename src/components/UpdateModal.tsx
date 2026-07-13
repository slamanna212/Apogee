import { Alert, Button, Group, Modal, Progress, Stack, Text } from '@mantine/core';
import { useUpdateStore } from '../stores/updateStore';

export function UpdateModal() {
  const { status, currentVersion, latestVersion, changelog, progress, errorMessage, downloadAndInstall, relaunchNow, dismiss } =
    useUpdateStore();

  if (status === 'idle' || status === 'checking') return null;

  const percent =
    progress?.total && progress.total > 0 ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100)) : undefined;

  return (
    <Modal opened onClose={dismiss} withCloseButton={false} size="440px" radius={20} centered>
      <Stack gap={14}>
        {status === 'available' && (
          <>
            <Text fw={700} size="lg" style={{ fontFamily: '"Space Grotesk", sans-serif' }}>
              Update available
            </Text>
            <Text size="sm" c="dimmed">
              {currentVersion ? `v${currentVersion} → v${latestVersion}` : `Version ${latestVersion} is available`}
            </Text>
            {changelog && (
              <Text size="sm" style={{ whiteSpace: 'pre-wrap', maxHeight: 200, overflowY: 'auto' }}>
                {changelog}
              </Text>
            )}
            <Group justify="flex-end" mt={8}>
              <Button variant="subtle" onClick={dismiss}>
                Not now
              </Button>
              <Button onClick={downloadAndInstall}>Download &amp; Install</Button>
            </Group>
          </>
        )}

        {status === 'downloading' && (
          <>
            <Text fw={700} size="lg" style={{ fontFamily: '"Space Grotesk", sans-serif' }}>
              Downloading update…
            </Text>
            <Progress value={percent ?? 100} animated={percent === undefined} striped={percent === undefined} />
            {percent !== undefined && (
              <Text size="xs" c="dimmed">
                {percent}%
              </Text>
            )}
          </>
        )}

        {status === 'ready' && (
          <>
            <Text fw={700} size="lg" style={{ fontFamily: '"Space Grotesk", sans-serif' }}>
              Update downloaded and installed
            </Text>
            <Text size="sm" c="dimmed">
              Restart Apogee to finish updating to v{latestVersion}.
            </Text>
            <Group justify="flex-end" mt={8}>
              <Button variant="subtle" onClick={dismiss}>
                Later
              </Button>
              <Button onClick={relaunchNow}>Restart &amp; install</Button>
            </Group>
          </>
        )}

        {status === 'error' && (
          <>
            <Alert color="red" title="Update failed">
              {errorMessage}
            </Alert>
            <Group justify="flex-end">
              <Button variant="subtle" onClick={dismiss}>
                Dismiss
              </Button>
            </Group>
          </>
        )}
      </Stack>
    </Modal>
  );
}
