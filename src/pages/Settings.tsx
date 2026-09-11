import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { Button, Group, Modal, Text } from '@mantine/core';
import { IconCheck } from '@tabler/icons-react';
import { SETTINGS_CATEGORIES, type SettingsCategoryId } from '../components/settings/categories';
import type { SettingsResetHandle } from '../components/settings/types';
import { ConnectionPanel } from '../components/settings/ConnectionPanel';
import { AppearancePanel } from '../components/settings/AppearancePanel';
import { AudioPanel } from '../components/settings/AudioPanel';
import { DiscordPanel } from '../components/settings/DiscordPanel';
import { ScrobblingPanel } from '../components/settings/ScrobblingPanel';
import { AlertsPanel } from '../components/settings/AlertsPanel';
import { UpdatesPanel } from '../components/settings/UpdatesPanel';
import { DiagnosticsPanel } from '../components/settings/DiagnosticsPanel';
import { AboutPanel } from '../components/settings/AboutPanel';

const SAVED_FLASH_MS = 1800;

function railItemStyle(active: boolean, hovered: boolean): CSSProperties {
  return {
    cursor: 'pointer',
    padding: '9px 10px',
    borderRadius: 10,
    fontSize: 13.5,
    fontFamily: "'Sora', sans-serif",
    background: active ? 'var(--app-accent-soft)' : hovered ? 'rgba(255,255,255,.05)' : 'transparent',
    color: active ? 'var(--app-text)' : 'var(--app-dim)',
    fontWeight: active ? 600 : 500,
  };
}

function ResetButton({ onClick }: { onClick: () => void }) {
  const [hovered, setHovered] = useState(false);
  return (
    <button
      onClick={onClick}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        flex: 'none',
        background: 'transparent',
        border: `1px solid ${hovered ? 'rgba(255,255,255,.28)' : 'rgba(255,255,255,.12)'}`,
        borderRadius: 10,
        padding: '7px 13px',
        color: hovered ? 'var(--app-text)' : 'var(--app-dim)',
        font: "600 12.5px 'Sora', sans-serif",
        cursor: 'pointer',
      }}
    >
      Reset to defaults
    </button>
  );
}

export function Settings() {
  const [category, setCategory] = useState<SettingsCategoryId>('connection');
  const [hoveredCategory, setHoveredCategory] = useState<SettingsCategoryId | null>(null);
  const [justSaved, setJustSaved] = useState(false);
  const [resetting, setResetting] = useState(false);
  const savedTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const resetHandleRef = useRef<SettingsResetHandle | null>(null);

  useEffect(() => () => {
    if (savedTimer.current) clearTimeout(savedTimer.current);
  }, []);

  function flashSaved() {
    setJustSaved(true);
    if (savedTimer.current) clearTimeout(savedTimer.current);
    savedTimer.current = setTimeout(() => setJustSaved(false), SAVED_FLASH_MS);
  }

  async function handleResetConfirm() {
    setResetting(false);
    await resetHandleRef.current?.reset();
    flashSaved();
  }

  const active = SETTINGS_CATEGORIES.find((c) => c.id === category) ?? SETTINGS_CATEGORIES[0];
  const railCategories = SETTINGS_CATEGORIES.filter((c) => c.id !== 'about');
  const aboutCategory = SETTINGS_CATEGORIES.find((c) => c.id === 'about')!;

  return (
    <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
      <div
        style={{
          flex: 'none',
          width: 208,
          borderRight: '1px solid var(--app-border)',
          padding: '26px 14px 84px',
          display: 'flex',
          flexDirection: 'column',
          gap: 3,
        }}
      >
        <div style={{ font: "700 18px 'Space Grotesk', sans-serif", padding: '0 10px', marginBottom: 14 }}>Settings</div>
        {railCategories.map((c) => (
          <div
            key={c.id}
            role="button"
            onClick={() => setCategory(c.id)}
            onMouseEnter={() => setHoveredCategory(c.id)}
            onMouseLeave={() => setHoveredCategory((h) => (h === c.id ? null : h))}
            style={railItemStyle(category === c.id, hoveredCategory === c.id)}
          >
            {c.label}
          </div>
        ))}
        <div style={{ flex: 1 }} />
        <div
          role="button"
          onClick={() => setCategory('about')}
          onMouseEnter={() => setHoveredCategory('about')}
          onMouseLeave={() => setHoveredCategory((h) => (h === 'about' ? null : h))}
          style={railItemStyle(category === 'about', hoveredCategory === 'about')}
        >
          {aboutCategory.label}
        </div>
      </div>

      <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}>
        <div style={{ flex: 1, minHeight: 0, overflowY: 'auto', padding: '26px 30px 84px' }}>
          <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 20 }}>
            <div>
              <div style={{ font: "700 20px 'Space Grotesk', sans-serif" }}>{active.title}</div>
              <div style={{ height: 18, marginTop: 5, display: 'flex', alignItems: 'center' }}>
                {justSaved ? (
                  <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 12.5, color: 'var(--app-accent2)' }}>
                    <IconCheck size={13} stroke={2.4} />
                    <span>Saved</span>
                  </div>
                ) : (
                  <div style={{ fontSize: 12.5, color: 'var(--app-dim2)' }}>{active.hint}</div>
                )}
              </div>
            </div>
            {!active.hideReset && <ResetButton onClick={() => setResetting(true)} />}
          </div>
          <div style={{ height: 1, background: 'var(--app-border)', margin: '18px 0 22px' }} />

          {category === 'connection' && <ConnectionPanel onSaved={flashSaved} />}
          {category === 'appearance' && <AppearancePanel ref={resetHandleRef} onSaved={flashSaved} />}
          {category === 'audio' && <AudioPanel ref={resetHandleRef} onSaved={flashSaved} />}
          {category === 'discord' && <DiscordPanel ref={resetHandleRef} onSaved={flashSaved} />}
          {category === 'scrobbling' && <ScrobblingPanel ref={resetHandleRef} onSaved={flashSaved} />}
          {category === 'alerts' && <AlertsPanel ref={resetHandleRef} onSaved={flashSaved} />}
          {category === 'updates' && <UpdatesPanel ref={resetHandleRef} onSaved={flashSaved} />}
          {category === 'diagnostics' && <DiagnosticsPanel ref={resetHandleRef} onSaved={flashSaved} />}
          {category === 'about' && <AboutPanel />}
        </div>
      </div>

      <Modal
        opened={resetting}
        onClose={() => setResetting(false)}
        withCloseButton={false}
        size="410px"
        radius={18}
        centered
        portalProps={{ target: '#apogee-window' }}
      >
        <Text fw={700} size="lg" mb={8} style={{ fontFamily: '"Space Grotesk", sans-serif' }}>
          Reset {active.title} to defaults?
        </Text>
        <Text size="sm" c="dimmed" mb={20}>
          Only this page is affected. Other settings stay as they are.
        </Text>
        <Group justify="flex-end">
          <Button variant="subtle" onClick={() => setResetting(false)}>
            Cancel
          </Button>
          <Button onClick={handleResetConfirm}>Reset page</Button>
        </Group>
      </Modal>
    </div>
  );
}
