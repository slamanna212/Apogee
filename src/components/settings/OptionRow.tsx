/** Renders a row of mutually-exclusive options as either an equal-width segmented control
 *  ("segment", used for Theme and Update channel - both have <=3 options where a dropdown
 *  would hide them for no reason) or a row of pill chips ("pill", used for Equalizer presets).
 *  Deliberately hand-styled rather than Mantine's SegmentedControl/Chip so the active/inactive
 *  colors come straight from the --app-* tokens the rest of the redesign uses. */
interface Option<T extends string> {
  value: T;
  label: string;
}

interface OptionRowProps<T extends string> {
  options: Option<T>[];
  value: T;
  onChange: (value: T) => void;
  shape?: 'segment' | 'pill';
  disabled?: boolean;
}

export function OptionRow<T extends string>({ options, value, onChange, shape = 'segment', disabled }: OptionRowProps<T>) {
  return (
    <div style={{ display: 'flex', gap: 8, flexWrap: shape === 'pill' ? 'wrap' : 'nowrap' }}>
      {options.map((opt) => {
        const active = opt.value === value;
        return (
          <div
            key={opt.value}
            role="button"
            aria-pressed={active}
            aria-disabled={disabled}
            tabIndex={disabled ? -1 : 0}
            onClick={() => { if (!disabled) onChange(opt.value); }}
            onKeyDown={(e) => {
              if (disabled) return;
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                onChange(opt.value);
              }
            }}
            style={{
              cursor: disabled ? 'default' : 'pointer',
              opacity: disabled ? 0.5 : 1,
              flex: shape === 'segment' ? 1 : 'none',
              textAlign: 'center',
              borderRadius: shape === 'segment' ? 10 : 999,
              padding: shape === 'segment' ? '9px 12px' : '6px 12px',
              fontFamily: "'Sora', sans-serif",
              fontSize: shape === 'segment' ? 13 : 12.5,
              fontWeight: 600,
              border: `1px solid ${active ? 'rgba(139,107,255,.55)' : 'rgba(255,255,255,.12)'}`,
              background: active ? 'var(--app-accent-soft)' : 'rgba(255,255,255,.03)',
              color: active ? 'var(--app-text)' : 'var(--app-dim)',
            }}
          >
            {opt.label}
          </div>
        );
      })}
    </div>
  );
}
