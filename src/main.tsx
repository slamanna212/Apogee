import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { attachConsole, error as logError } from '@tauri-apps/plugin-log'
import '@mantine/core/styles.css'
import '@mantine/notifications/styles.css'
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

function describeError(value: unknown): string {
  if (value instanceof Error) return value.stack || value.message
  return String(value)
}

function persistFrontendError(message: string): void {
  // Avoid turning a logging-plugin failure during teardown into another
  // unhandled rejection (which would otherwise recursively log itself).
  void logError(message).catch(() => undefined)
}

// React reports render failures through the root callback instead of the
// browser's global `error` event. Persist both kinds (plus rejected promises)
// so an exported log contains the actual frontend failure rather than ending
// at the last unrelated native/mpv message before the user restarts the app.
window.addEventListener('error', (event) => {
  persistFrontendError(`uncaught frontend error: ${describeError(event.error ?? event.message)}`)
})
window.addEventListener('unhandledrejection', (event) => {
  persistFrontendError(`unhandled frontend rejection: ${describeError(event.reason)}`)
})

createRoot(document.getElementById('root')!, {
  onUncaughtError(error, errorInfo) {
    persistFrontendError(`uncaught React error: ${describeError(error)}${errorInfo.componentStack ?? ''}`)
  },
}).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
