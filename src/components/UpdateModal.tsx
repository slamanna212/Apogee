import { Alert, Button, Group, Modal, Stack, Text } from '@mantine/core';
import { useUpdateStore } from '../stores/updateStore';

// Mantine's <Progress> drives its fill width through a CSS custom property
// (--progress-section-size), which - under the rapid stream of state updates
// a large download produces - has been observed to leave the fill visually
// frozen while the percentage text keeps climbing (a WebView2 repaint quirk
// with CSS-var-driven widths, not a state bug: both come from the same
// `percent` value in the same render). A directly-styled width avoids that
// indirection entirely, matching the track/fill pattern already used for the
// volume slider in TransportBar.tsx.
function DownloadProgressBar({ percent }: { percent: number | undefined }) {
  return (
    <div
      style={{
        width: '100%',
        height: 6,
        borderRadius: 4,
        background: 'rgba(255,255,255,.12)',
        overflow: 'hidden',
      }}
    >
      <div
        style={{
          width: percent !== undefined ? `${percent}%` : '100%',
          height: '100%',
          borderRadius: 4,
          background: 'var(--app-accent2)',
          transition: 'width 100ms linear',
        }}
      />
    </div>
  );
}

export function UpdateModal() {
  const { status, currentVersion, latestVersion, changelog, progress, errorMessage, downloadAndInstall, relaunchNow, dismiss } =
    useUpdateStore();

  if (status === 'idle' || status === 'checking') return null;

  const percent =
    progress?.total && progress.total > 0 ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100)) : undefined;

  return (
    <Modal
      opened
      onClose={dismiss}
      withCloseButton={false}
      size="440px"
      radius={20}
      centered
      portalProps={{ target: '#apogee-window' }}
    >
      <Stack gap={14}>
        {status === 'available' && (
          <>
            <Text fw={700} size="lg" style={{ fontFamily: '"Space Grotesk", sans-serif' }}>
              Update available
            </Text>
            <Text size="sm" c="dimmed">
              {currentVersion ? `v${currentVersion} → v${latestVersion}` : `Version ${latestVersion} is available`}
            </Text>
            {changelog.length > 0 && (
              <Stack gap={10} style={{ maxHeight: 200, overflowY: 'auto' }}>
                {changelog.map((entry) => (
                  <div key={entry.version}>
                    <Text size="sm" fw={600}>
                      v{entry.version}
                    </Text>
                    <Text size="sm" c="dimmed" style={{ whiteSpace: 'pre-wrap' }}>
                      {entry.body?.trim() || 'No release notes provided.'}
                    </Text>
                  </div>
                ))}
              </Stack>
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
            <DownloadProgressBar percent={percent} />
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
