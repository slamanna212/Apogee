import { createTheme, type CSSVariablesResolver, type MantineColorsTuple } from '@mantine/core';

const violet: MantineColorsTuple = [
  '#f1edff',
  '#e1d9ff',
  '#c3b3ff',
  '#a58cff',
  '#8b6bff',
  '#7c56ff',
  '#7249f0',
  '#6239d6',
  '#5730bd',
  '#4b28a3',
];

export const theme = createTheme({
  primaryColor: 'violet',
  colors: { violet },
  fontFamily: "'Sora', sans-serif",
  headings: { fontFamily: "'Space Grotesk', sans-serif" },
  radius: { sm: '10px', md: '18px', lg: '28px' },
  defaultRadius: 'md',
});

/**
 * Custom surfaces (transport bar, channel cards, modal) read from these
 * --app-* variables instead of hardcoded colors, so they follow Mantine's
 * colorScheme the same way native Mantine components do.
 */
export const cssVariablesResolver: CSSVariablesResolver = () => ({
  variables: {},
  light: {
    '--app-bg': '#f2eff9',
    '--app-bg2': '#ffffff',
    '--app-panel': '#ffffff',
    '--app-panel2': 'rgba(20,16,40,0.05)',
    '--app-border': 'rgba(20,16,40,0.1)',
    '--app-text': '#1a1730',
    '--app-dim': 'rgba(26,23,48,0.62)',
    '--app-dim2': 'rgba(26,23,48,0.5)',
    '--app-accent': '#6c47ff',
    '--app-accent-soft': 'rgba(108,71,255,0.14)',
    '--app-accent2': '#20b8c9',
    '--app-accent2-soft': 'rgba(32,184,201,0.15)',
    '--app-shadow': '0 8px 20px rgba(90,60,180,.16), 0 2px 5px rgba(90,60,180,.1)',
    '--app-shadow-card': '0 1px 2px rgba(90,60,180,.08), 0 6px 14px -6px rgba(90,60,180,.14), 0 12px 24px -12px rgba(90,60,180,.14)',
    '--app-shadow-rail': '0 1px 2px rgba(90,60,180,.10), 0 6px 12px -4px rgba(90,60,180,.14), 0 16px 24px -14px rgba(90,60,180,.16)',
    '--app-glass-bg': 'linear-gradient(145deg, #ffffff 0%, #f8f6fd 58%, #eeebf7 100%)',
    '--app-glass-highlight': 'rgba(255,255,255,0.82)',
    '--app-glass-noise-opacity': '0.025',
  },
  dark: {
    '--app-bg': '#07060d',
    '--app-bg2': '#100e1c',
    '--app-panel': 'rgba(255,255,255,0.05)',
    '--app-panel2': 'rgba(255,255,255,0.08)',
    '--app-border': 'rgba(255,255,255,0.09)',
    '--app-text': '#f6f5fb',
    '--app-dim': 'rgba(246,245,251,0.6)',
    '--app-dim2': 'rgba(246,245,251,0.36)',
    '--app-accent': '#8b6bff',
    '--app-accent-soft': 'rgba(139,107,255,0.16)',
    '--app-accent2': '#45e0d8',
    '--app-accent2-soft': 'rgba(69,224,216,0.15)',
    '--app-shadow': '0 8px 20px rgba(0,0,0,.35), 0 2px 5px rgba(0,0,0,.25)',
    '--app-shadow-card': '0 1px 2px rgba(0,0,0,.3), 0 6px 14px -6px rgba(0,0,0,.35), 0 12px 24px -12px rgba(0,0,0,.35)',
    '--app-shadow-rail': '0 1px 2px rgba(0,0,0,.35), 0 6px 12px -4px rgba(0,0,0,.45), 0 16px 24px -14px rgba(0,0,0,.4)',
    '--app-glass-bg': 'linear-gradient(145deg, #211e31 0%, #151321 58%, #0d0c14 100%)',
    '--app-glass-highlight': 'rgba(255,255,255,0.13)',
    '--app-glass-noise-opacity': '0.04',
  },
});
