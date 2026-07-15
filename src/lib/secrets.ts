import { invoke } from '@tauri-apps/api/core';

export function setSecret(key: string, value: string): Promise<void> {
  return invoke('secrets_set', { key, value });
}

export function getSecret(key: string): Promise<string | null> {
  return invoke('secrets_get', { key });
}

export function deleteSecret(key: string): Promise<void> {
  return invoke('secrets_delete', { key });
}

export function getBuiltinStellarApiKey(): Promise<string | null> {
  return invoke('secrets_get_builtin_stellar_key');
}

export const SECRET_KEYS = {
  xtreamPassword: 'xtream_password',
} as const;
