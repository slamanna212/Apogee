import { useEffect, useMemo, useRef, useState } from 'react';
import { ActionIcon, Button, Text } from '@mantine/core';
import { IconMicrophone2, IconMusic, IconTrash } from '@tabler/icons-react';
import { useAlertsStore } from '../stores/alertsStore';
import { useChannelStore } from '../stores/channelStore';
import { useLibraryStore } from '../stores/libraryStore';
import { matchesEntry } from '../lib/songMatcher';
import { ChannelActionsMenu } from '../components/ChannelActionsMenu';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarStation } from '../types/stellarTunerLog';
import type { AlertEntry } from '../types/alerts';

function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ background: 'var(--app-panel)', border: '1px solid var(--app-border)', borderRadius: 16, padding: 20 }}>
      <Text size="sm" fw={600} c="dimmed" mb={14}>
        {title}
      </Text>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>{children}</div>
    </div>
  );
}

function NowPlayingCard({
  channel,
  metadata,
  station,
  isFavorite,
  onToggleFavorite,
  onTune,
}: {
  channel: XtreamChannel;
  metadata?: StellarChannel;
  station: StellarStation;
  isFavorite: boolean;
  onToggleFavorite: () => void;
  onTune: () => void;
}) {
  const logoUrl = metadata?.logos?.color_dark_square?.url || channel.stream_icon;
  const name = metadata?.marketing_name || channel.name;
  const artworkUrl = station.artwork_url;

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        background: 'var(--app-panel2)',
        border: '1px solid var(--app-border)',
        borderRadius: 14,
        padding: '10px 14px',
      }}
    >
      <div style={{ width: 44, height: 44, flex: 'none', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        {logoUrl && <img src={logoUrl} alt="" style={{ width: '100%', height: '100%', objectFit: 'contain' }} />}
      </div>
      {artworkUrl ? (
        <img
          src={artworkUrl}
          alt=""
          style={{ width: 52, height: 52, flex: 'none', borderRadius: 12, objectFit: 'cover', background: 'var(--app-panel2)' }}
        />
      ) : (
        <div style={{ width: 52, height: 52, flex: 'none', borderRadius: 12, background: 'var(--app-panel2)' }} />
      )}
      <div style={{ flex: 1, minWidth: 0 }}>
        <Text size="sm" fw={700} style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {station.title}
        </Text>
        <Text size="xs" c="dimmed" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {station.artist} — {name}
        </Text>
      </div>
      <ChannelActionsMenu
        nowPlaying={station}
        isFavorite={isFavorite}
        onToggleFavorite={onToggleFavorite}
        triggerStyle={{
          width: 32,
          height: 32,
          flex: 'none',
          borderRadius: '50%',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: isFavorite ? 'var(--app-accent)' : 'var(--app-dim)',
        }}
      />
      <Button size="xs" variant="light" onClick={onTune}>
        Tune
      </Button>
    </div>
  );
}

function FollowCard({ entry, onUnfollow }: { entry: AlertEntry; onUnfollow: () => void }) {
  const Icon = entry.type === 'artist' ? IconMicrophone2 : IconMusic;
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        background: 'var(--app-panel2)',
        border: '1px solid var(--app-border)',
        borderRadius: 14,
        padding: '10px 14px',
      }}
    >
      <div
        style={{
          width: 36,
          height: 36,
          flex: 'none',
          borderRadius: 10,
          background: 'var(--app-accent-soft)',
          color: 'var(--app-accent)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        <Icon size={18} />
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <Text size="sm" fw={600} style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {entry.type === 'artist' ? entry.artist : entry.title}
        </Text>
        {entry.type === 'track' && (
          <Text size="xs" c="dimmed" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {entry.artist}
          </Text>
        )}
      </div>
      <ActionIcon variant="subtle" color="red" onClick={onUnfollow} aria-label="Unfollow">
        <IconTrash size={16} />
      </ActionIcon>
    </div>
  );
}

interface CurrentMatch {
  channel: XtreamChannel;
  station: StellarStation;
}

interface AlertsProps {
  onPlayChannel: (streamId: number) => void;
}

export function Alerts({ onPlayChannel }: AlertsProps) {
  const entries = useAlertsStore((s) => s.entries);
  const unfollow = useAlertsStore((s) => s.unfollow);
  const channels = useChannelStore((s) => s.channels);
  const channelMetadata = useChannelStore((s) => s.channelMetadata);
  const nowPlaying = useChannelStore((s) => s.nowPlaying);
  const favorites = useLibraryStore((s) => s.favorites);
  const toggleFavorite = useLibraryStore((s) => s.toggleFavorite);

  const [followTab, setFollowTab] = useState<'tracks' | 'artists'>('tracks');
  const initializedTabRef = useRef(false);

  const currentMatches = useMemo(() => {
    const matches: CurrentMatch[] = [];
    for (const [streamId, station] of nowPlaying) {
      const channel = channels.find((c) => c.stream_id === streamId);
      if (!channel) continue;
      if (!entries.some((e) => matchesEntry(station, e))) continue;
      matches.push({ channel, station });
    }
    return matches;
  }, [entries, nowPlaying, channels]);

  const trackEntries = useMemo(() => entries.filter((e) => e.type === 'track'), [entries]);
  const artistEntries = useMemo(() => entries.filter((e) => e.type === 'artist'), [entries]);
  const activeEntries = followTab === 'tracks' ? trackEntries : artistEntries;

  // Once entries first load, land on whichever tab actually has followed
  // entries rather than defaulting to a possibly-empty "Tracks" tab.
  useEffect(() => {
    if (initializedTabRef.current || entries.length === 0) return;
    initializedTabRef.current = true;
    if (trackEntries.length === 0 && artistEntries.length > 0) setFollowTab('artists');
  }, [entries.length, trackEntries.length, artistEntries.length]);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
      <div style={{ font: '700 24px "Space Grotesk", sans-serif', marginBottom: 24, width: '100%', maxWidth: 600 }}>Alerts</div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 16, width: '100%', maxWidth: 600 }}>
        <Card title="Currently playing">
          {currentMatches.length === 0 ? (
            <Text c="dimmed">None of your followed tracks or artists are playing right now.</Text>
          ) : (
            currentMatches.map(({ channel, station }) => (
              <NowPlayingCard
                key={channel.stream_id}
                channel={channel}
                metadata={channelMetadata.get(channel.stream_id)}
                station={station}
                isFavorite={favorites.includes(channel.stream_id)}
                onToggleFavorite={() => toggleFavorite(channel.stream_id)}
                onTune={() => onPlayChannel(channel.stream_id)}
              />
            ))
          )}
        </Card>

        <Card title="Following">
          {entries.length === 0 ? (
            <Text c="dimmed">
              Nothing followed yet — use the menu on the transport bar while something is playing to follow a track or artist.
            </Text>
          ) : (
            <>
              <div
                style={{
                  display: 'flex',
                  width: 'fit-content',
                  alignSelf: 'center',
                  background: 'var(--app-panel2)',
                  border: '1px solid var(--app-border)',
                  borderRadius: 999,
                  padding: 3,
                }}
              >
                {(['tracks', 'artists'] as const).map((tab) => (
                  <div
                    key={tab}
                    onClick={() => setFollowTab(tab)}
                    role="button"
                    style={{
                      padding: '6px 14px',
                      borderRadius: 999,
                      cursor: 'pointer',
                      background: followTab === tab ? 'var(--app-accent)' : 'transparent',
                      color: followTab === tab ? 'var(--app-bg)' : 'var(--app-dim)',
                      font: '600 12px "Sora", sans-serif',
                    }}
                  >
                    {tab === 'tracks' ? `Tracks (${trackEntries.length})` : `Artists (${artistEntries.length})`}
                  </div>
                ))}
              </div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 10, marginTop: 14 }}>
                {activeEntries.length === 0 ? (
                  <Text c="dimmed">{followTab === 'tracks' ? 'No tracks followed yet.' : 'No artists followed yet.'}</Text>
                ) : (
                  activeEntries.map((entry) => <FollowCard key={entry.id} entry={entry} onUnfollow={() => unfollow(entry.id)} />)
                )}
              </div>
            </>
          )}
        </Card>
      </div>
    </div>
  );
}
