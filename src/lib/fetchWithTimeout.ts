import { fetch } from '@tauri-apps/plugin-http';

const DEFAULT_TIMEOUT_MS = 10_000;

/**
 * @tauri-apps/plugin-http's fetch has no timeout of its own, so a stalled
 * upstream (Xtream server, StellarTunerLog) leaves the promise pending
 * indefinitely with no way for a caller to cancel it. Wraps it with an
 * AbortController-based timeout instead.
 */
export function fetchWithTimeout(
  input: string,
  init: RequestInit = {},
  timeoutMs: number = DEFAULT_TIMEOUT_MS,
): Promise<Response> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  return fetch(input, { ...init, signal: controller.signal }).finally(() => clearTimeout(timer));
}
