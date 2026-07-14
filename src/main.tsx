import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { attachConsole } from '@tauri-apps/plugin-log'
import '@mantine/core/styles.css'
import '@fontsource/space-grotesk/500.css'
import '@fontsource/space-grotesk/600.css'
import '@fontsource/space-grotesk/700.css'
import '@fontsource/sora/400.css'
import '@fontsource/sora/500.css'
import '@fontsource/sora/600.css'
import './index.css'
import App from './App.tsx'

// Mirrors Rust-side log entries into the webview's own devtools console, for
// live viewing during development. Persisting frontend-side diagnostics the
// other direction (JS -> the exportable log file) is done explicitly via
// @tauri-apps/plugin-log's info/warn/error/debug functions at each call
// site (see playerStore.ts, App.tsx) - plain console.* calls never reach it.
attachConsole()

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
