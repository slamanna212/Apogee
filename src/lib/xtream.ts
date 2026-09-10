import { invoke } from '@tauri-apps/api/core';
import type { XtreamCategory, XtreamChannel } from '../types/xtream';

export interface XtreamCredentials {
  baseUrl: string;
  username: string;
  password: string;
}

/**
 * Xtream API calls run in Rust (`src-tauri/src/xtream.rs`) so all application HTTP
 * shares one client, TLS policy and redaction path.
 *
 * Rust builds the request URL, which matters because the credentials travel in query
 * parameters: an error message formatted from a transport failure would otherwise embed
 * them, and these messages are surfaced directly in UI-visible store state.
 */
export function getLiveCategories(creds: XtreamCredentials): Promise<XtreamCategory[]> {
  return invoke('xtream_get_live_categories', { creds });
}

export function getLiveStreams(
  creds: XtreamCredentials,
  categoryId: string,
): Promise<XtreamChannel[]> {
  return invoke('xtream_get_live_streams', { creds, categoryId });
}
