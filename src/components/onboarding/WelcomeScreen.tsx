import { Button, Text } from '@mantine/core';
import logoUrl from '../../assets/logo.svg';

export function WelcomeScreen({ onNext }: { onNext: () => void }) {
  return (
    <div
      style={{
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 20,
        textAlign: 'center',
        maxWidth: 460,
      }}
    >
      <img src={logoUrl} alt="Apogee" width={140} height={140} style={{ animation: 'floaty 6s ease-in-out infinite' }} />
      <div style={{ font: '700 30px "Space Grotesk", sans-serif' }}>Welcome to Apogee</div>
      <Text size="sm" c="dimmed" style={{ lineHeight: 1.6 }}>
        Apogee turns your Xtream Codes IPTV subscription into a dedicated satellite radio tuner. Along the way it
        pulls live song and artist info so you always know what's playing.
      </Text>
      <Text size="sm" c="dimmed" style={{ lineHeight: 1.6 }}>
        This quick setup will connect your Xtream account, optionally hook up now-playing data, and show you around
        the app's controls. It only takes a minute.
      </Text>
      <Button size="md" onClick={onNext} mt={8}>
        Get started
      </Button>
    </div>
  );
}
