import { useEffect, useRef, useState } from 'react';
import { Modal, Text } from '@mantine/core';
import { IconBrandFacebook, IconBrandTwitter, IconMail, IconPhone, IconStar, IconStarFilled } from '@tabler/icons-react';
import type { XtreamChannel } from '../types/xtream';
import type { StellarChannel, StellarHistoryEntry } from '../types/stellarTunerLog';
import { getHistory } from '../lib/stellarTunerLog';

interface ChannelModalProps {
  channel: XtreamChannel;
  metadata?: StellarChannel;
  apiKey: string;
  isFavorite: boolean;
  onToggleFavorite: () => void;
  onClose: () => void;
}

const BATCH_SIZE = 10;

export function ChannelModal({ channel, metadata, apiKey, isFavorite, onToggleFavorite, onClose }: ChannelModalProps) {
  const [history, setHistory] = useState<StellarHistoryEntry[]>([]);
  const [visibleCount, setVisibleCount] = useState(BATCH_SIZE);
  const scrollRef = useRef<HTMLDivElement>(null);
  const throttledRef = useRef(false);

  useEffect(() => {
    setHistory([]);
    setVisibleCount(BATCH_SIZE);
    if (!metadata || !apiKey) return;
    let cancelled = false;
    getHistory(metadata.id, apiKey)
      .then((plays) => {
        if (!cancelled) setHistory(plays);
      })
      .catch(() => {
        if (!cancelled) setHistory([]);
      });
    return () => {
      cancelled = true;
    };
  }, [metadata, apiKey]);

  function handleScroll() {
    if (throttledRef.current) return;
    throttledRef.current = true;
    setTimeout(() => {
      throttledRef.current = false;
    }, 200);
    const el = scrollRef.current;
    if (!el) return;
    if (el.scrollTop + el.clientHeight > el.scrollHeight - 80) {
      setVisibleCount((v) => Math.min(history.length, v + BATCH_SIZE));
    }
  }

  const name = metadata?.marketing_name || channel.name;
  const description = metadata?.medium_description || metadata?.long_description || metadata?.streaming_name || '';
  const categories = [...(metadata?.categories ?? [])].sort((a, b) => a.order - b.order);
  const primary = categories.find((c) => c.is_primary);
  const rest = categories.filter((c) => c !== primary);
  const socials = [
    metadata?.twitter && { Icon: IconBrandTwitter, href: metadata.twitter },
    metadata?.facebook && { Icon: IconBrandFacebook, href: metadata.facebook },
    metadata?.email && { Icon: IconMail, href: `mailto:${metadata.email}` },
    metadata?.phone && { Icon: IconPhone, href: `tel:${metadata.phone}` },
  ].filter((s): s is { Icon: typeof IconMail; href: string } => Boolean(s));
  const backdrop =
    metadata?.logos?.background_wide?.url ||
    metadata?.logos?.background_thumb?.url ||
    metadata?.logos?.background_square?.url ||
    metadata?.logos?.background?.url;
  const backdropColor = metadata?.dark_bg_color || '#111116';

  return (
    <Modal opened onClose={onClose} withCloseButton={false} size="1100px" radius={26} padding={0} centered>
      <div style={{ display: 'flex', height: 560 }}>
        <div
          style={{
            flex: '0 0 60%',
            position: 'relative',
            overflow: 'hidden',
            padding: 36,
            display: 'flex',
            flexDirection: 'column',
            color: '#fff',
            background: backdropColor,
          }}
        >
          {backdrop && (
            <img
              src={backdrop}
              alt=""
              style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', objectFit: 'cover', filter: 'blur(14px)', transform: 'scale(1.1)' }}
            />
          )}
          <div style={{ position: 'absolute', inset: 0, background: 'rgba(7,6,13,.5)' }} />
          <div style={{ position: 'relative', display: 'flex', justifyContent: 'flex-end', gap: 10 }}>
            <div
              onClick={onToggleFavorite}
              role="button"
              aria-label={isFavorite ? 'Remove favorite' : 'Add favorite'}
              style={{
                width: 32,
                height: 32,
                borderRadius: '50%',
                background: 'rgba(255,255,255,.1)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                cursor: 'pointer',
                color: isFavorite ? 'var(--app-accent)' : '#fff',
              }}
            >
              {isFavorite ? <IconStarFilled size={15} /> : <IconStar size={15} />}
            </div>
            <div
              onClick={onClose}
              role="button"
              aria-label="Close"
              style={{
                width: 32,
                height: 32,
                borderRadius: '50%',
                background: 'rgba(255,255,255,.1)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                cursor: 'pointer',
              }}
            >
              ×
            </div>
          </div>
          <div style={{ position: 'relative', flex: 1, display: 'flex', flexDirection: 'column', justifyContent: 'center' }}>
            <div style={{ font: '700 40px "Space Grotesk", sans-serif', marginBottom: 14 }}>{name}</div>
            {(primary || rest.length > 0) && (
              <div style={{ display: 'flex', gap: 8, marginBottom: 18, flexWrap: 'wrap' }}>
                {primary && (
                  <span
                    style={{
                      font: '700 11px "Space Grotesk", sans-serif',
                      background: 'var(--app-accent)',
                      color: '#fff',
                      borderRadius: 999,
                      padding: '6px 14px',
                    }}
                  >
                    {primary.name}
                  </span>
                )}
                {rest.map((c) => (
                  <span
                    key={c.name}
                    style={{
                      font: '600 11px "Space Grotesk", sans-serif',
                      background: 'rgba(255,255,255,.08)',
                      border: '1px solid rgba(255,255,255,.18)',
                      color: '#fff',
                      borderRadius: 999,
                      padding: '6px 14px',
                    }}
                  >
                    {c.name}
                  </span>
                ))}
              </div>
            )}
            {description && (
              <div style={{ font: '400 14.5px/1.7 "Sora", sans-serif', color: 'rgba(255,255,255,.78)', maxWidth: 440, marginBottom: 24 }}>
                {description}
              </div>
            )}
            {socials.length > 0 && (
              <div style={{ display: 'flex', gap: 12 }}>
                {socials.map(({ Icon, href }, i) => (
                  <a
                    key={i}
                    href={href}
                    target="_blank"
                    rel="noreferrer"
                    style={{
                      width: 36,
                      height: 36,
                      borderRadius: '50%',
                      background: 'rgba(255,255,255,.1)',
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'center',
                      color: '#fff',
                    }}
                  >
                    <Icon size={16} />
                  </a>
                ))}
              </div>
            )}
          </div>
        </div>
        <div style={{ flex: '0 0 40%', background: 'var(--app-bg2)', padding: 30, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
          <div style={{ font: '700 18px "Space Grotesk", sans-serif', marginBottom: 16 }}>Recently played</div>
          <div ref={scrollRef} onScroll={handleScroll} style={{ display: 'flex', flexDirection: 'column', gap: 10, overflowY: 'auto' }}>
            {history.slice(0, visibleCount).map((entry, i) => (
              <div
                key={i}
                style={{
                  display: 'flex',
                  gap: 12,
                  alignItems: 'center',
                  background: 'var(--app-panel)',
                  border: '1px solid var(--app-border)',
                  borderRadius: 14,
                  padding: 10,
                }}
              >
                <div style={{ width: 44, height: 44, borderRadius: 8, background: 'var(--app-panel2)', flex: 'none', overflow: 'hidden' }}>
                  {entry.artwork_url && (
                    <img src={entry.artwork_url} alt="" style={{ width: '100%', height: '100%', objectFit: 'cover' }} />
                  )}
                </div>
                <div style={{ minWidth: 0 }}>
                  <Text size="sm" fw={600} truncate>
                    {entry.title}
                  </Text>
                  <Text size="xs" c="dimmed" truncate>
                    {[entry.artist, entry.album].filter(Boolean).join(' — ')}
                  </Text>
                </div>
              </div>
            ))}
            {history.length === 0 && (
              <Text size="sm" c="dimmed">
                No recent history available.
              </Text>
            )}
          </div>
        </div>
      </div>
    </Modal>
  );
}
