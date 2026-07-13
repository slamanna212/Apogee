import { useState } from 'react';
import { useSettingsStore } from '../../stores/settingsStore';
import type { Settings } from '../../stores/settingsStore';
import { WelcomeScreen } from './WelcomeScreen';
import { XtreamSetupScreen } from './XtreamSetupScreen';
import { StellarApiScreen } from './StellarApiScreen';
import { ControlsGuideScreen } from './ControlsGuideScreen';

type OnboardingStep = 0 | 1 | 2 | 3;

const STEP_LABELS = ['Welcome', 'Xtream connection', 'StellarTunerLog', 'Get around'];

function clampStep(step: number): OnboardingStep {
  if (step < 0 || step > 3 || Number.isNaN(step)) return 0;
  return step as OnboardingStep;
}

function StepIndicator({ step }: { step: OnboardingStep }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 8, marginBottom: 8, flex: 'none' }}>
      <div style={{ display: 'flex', gap: 8 }}>
        {STEP_LABELS.map((label, i) => (
          <div
            key={label}
            style={{
              width: i === step ? 22 : 7,
              height: 7,
              borderRadius: 999,
              background: i <= step ? 'var(--app-accent)' : 'var(--app-panel2)',
              transition: 'width 200ms, background 200ms',
            }}
          />
        ))}
      </div>
      <div style={{ font: '600 12px "Sora", sans-serif', color: 'var(--app-dim)' }}>
        Step {step + 1} of {STEP_LABELS.length} — {STEP_LABELS[step]}
      </div>
    </div>
  );
}

export function OnboardingWizard() {
  const settings = useSettingsStore((s) => s.settings);
  const updateSettings = useSettingsStore((s) => s.update);
  const [step, setStep] = useState<OnboardingStep>(() => clampStep(settings.onboardingStep));

  function advance(patch: Partial<Settings>, nextStep: OnboardingStep) {
    void updateSettings({ ...patch, onboardingStep: nextStep });
    setStep(nextStep);
  }

  function back() {
    const prevStep = clampStep(step - 1);
    void updateSettings({ onboardingStep: prevStep });
    setStep(prevStep);
  }

  function finish(patch: Partial<Settings>) {
    void updateSettings({ ...patch, onboardingComplete: true, onboardingStep: 3 });
  }

  return (
    <div style={{ position: 'relative', flex: 1, minHeight: 0, overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
      <div className="apogee-starfield">
        <div className="apogee-starfield-dots" />
      </div>
      <div
        style={{
          position: 'relative',
          zIndex: 1,
          flex: 1,
          minHeight: 0,
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          overflowY: 'auto',
          padding: '28px 32px 36px',
          color: 'var(--app-text)',
        }}
      >
        <StepIndicator step={step} />
        {step === 0 && <WelcomeScreen onNext={() => advance({}, 1)} />}
        {step === 1 && (
          <XtreamSetupScreen settings={settings} onBack={back} onNext={(patch) => advance(patch, 2)} />
        )}
        {step === 2 && (
          <StellarApiScreen
            settings={settings}
            onBack={back}
            onSkip={(patch) => advance(patch, 3)}
            onNext={(patch) => advance(patch, 3)}
          />
        )}
        {step === 3 && <ControlsGuideScreen onBack={back} onFinish={() => finish({})} />}
      </div>
    </div>
  );
}
