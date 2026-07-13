import { useState } from 'react';
import { Alert, Button, Group, PasswordInput, Select, Text, TextInput } from '@mantine/core';
import type { Settings } from '../../stores/settingsStore';
import { getLiveCategories } from '../../lib/xtream';
import type { XtreamCategory } from '../../types/xtream';
import { OnboardingCard } from './OnboardingCard';

interface XtreamSetupScreenProps {
  settings: Settings;
  onBack: () => void;
  onNext: (patch: Partial<Settings>) => void;
}

export function XtreamSetupScreen({ settings, onBack, onNext }: XtreamSetupScreenProps) {
  const [baseUrl, setBaseUrl] = useState(settings.baseUrl);
  const [username, setUsername] = useState(settings.username);
  const [password, setPassword] = useState(settings.password);
  const [categories, setCategories] = useState<XtreamCategory[]>([]);
  const [categoryId, setCategoryId] = useState<string | null>(settings.categoryId);
  const [testStatus, setTestStatus] = useState<'idle' | 'testing' | 'ok' | 'error'>('idle');
  const [testError, setTestError] = useState<string | null>(null);

  async function handleTestConnection() {
    setTestStatus('testing');
    setTestError(null);
    try {
      const cats = await getLiveCategories({ baseUrl, username, password });
      setCategories(cats);
      setTestStatus('ok');
    } catch (err) {
      setTestStatus('error');
      setTestError(err instanceof Error ? err.message : String(err));
    }
  }

  function handleNext() {
    const category = categories.find((c) => c.category_id === categoryId);
    onNext({
      baseUrl,
      username,
      password,
      categoryId,
      categoryName: category?.category_name ?? settings.categoryName,
    });
  }

  return (
    <div style={{ flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 16, width: '100%', maxWidth: 520 }}>
      <div style={{ textAlign: 'center' }}>
        <div style={{ font: '700 22px "Space Grotesk", sans-serif', marginBottom: 6 }}>Connect your Xtream account</div>
        <Text size="sm" c="dimmed">
          Enter the Xtream Codes details from your IPTV provider, then pick the channel group that contains your
          SiriusXM channels.
        </Text>
      </div>

      <OnboardingCard title="Xtream connection">
        <TextInput label="Xtream base URL" placeholder="http://host:port" value={baseUrl} onChange={(e) => setBaseUrl(e.currentTarget.value)} />
        <Group grow>
          <TextInput label="Username" value={username} onChange={(e) => setUsername(e.currentTarget.value)} />
          <PasswordInput label="Password" value={password} onChange={(e) => setPassword(e.currentTarget.value)} />
        </Group>
        <Group align="center">
          <Button onClick={handleTestConnection} loading={testStatus === 'testing'}>
            Test connection
          </Button>
          {testStatus === 'ok' && categories.length > 0 && (
            <Text c="teal" size="sm">
              Connected — {categories.length} categories found
            </Text>
          )}
        </Group>
        {testStatus === 'error' && (
          <Alert color="red" title="Connection failed">
            {testError}
          </Alert>
        )}
        <Select
          label="Channel group"
          placeholder="Run Test connection to load groups"
          data={categories.map((c) => ({ value: c.category_id, label: c.category_name }))}
          value={categoryId}
          onChange={setCategoryId}
          disabled={categories.length === 0}
          searchable
        />
      </OnboardingCard>

      <Group justify="space-between" w="100%" mt={4}>
        <Button variant="subtle" onClick={onBack}>
          Back
        </Button>
        <Button onClick={handleNext} disabled={!categoryId}>
          Next
        </Button>
      </Group>
    </div>
  );
}
