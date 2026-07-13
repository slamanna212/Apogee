import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { fetch } from '@tauri-apps/plugin-http';
import { Update, type DownloadEvent } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import type { UpdateChannel } from './settingsStore';

const REPO = 'slamanna212/Apogee';

export type UpdateStatus = 'idle' | 'checking' | 'available' | 'downloading' | 'ready' | 'error';

interface DownloadProgress {
  downloaded: number;
  total?: number;
}

interface GithubReleaseAsset {
  name: string;
  browser_download_url: string;
}

interface GithubRelease {
  draft: boolean;
  prerelease: boolean;
  assets: GithubReleaseAsset[];
}

interface UpdateMetadata {
  rid: number;
  currentVersion: string;
  version: string;
  date?: string;
  body?: string;
  rawJson: Record<string, unknown>;
}

interface UpdateState {
  status: UpdateStatus;
  currentVersion?: string;
  latestVersion?: string;
  changelog?: string;
  progress?: DownloadProgress;
  errorMessage?: string;
  pendingUpdate: Update | null;
  checkForUpdates: (channel: UpdateChannel) => Promise<void>;
  downloadAndInstall: () => Promise<void>;
  relaunchNow: () => Promise<void>;
  dismiss: () => void;
}

// GitHub has no "latest release including prereleases" URL alias, so the
// channel is resolved here by walking the (newest-first, draft-free for
// unauthenticated requests) release list ourselves rather than relying on
// the static endpoint baked into tauri.conf.json.
async function findLatestJsonUrl(channel: UpdateChannel): Promise<string | null> {
  const response = await fetch(`https://api.github.com/repos/${REPO}/releases`, {
    headers: { Accept: 'application/vnd.github+json' },
  });
  if (!response.ok) {
    throw new Error(`GitHub API request failed: ${response.status}`);
  }
  const releases = (await response.json()) as GithubRelease[];
  const release = releases.find((r) => !r.draft && (channel === 'beta' || !r.prerelease));
  if (!release) return null;

  const asset = release.assets.find((a) => a.name === 'latest.json');
  return asset?.browser_download_url ?? null;
}

export const useUpdateStore = create<UpdateState>((set, get) => ({
  status: 'idle',
  pendingUpdate: null,

  async checkForUpdates(channel) {
    set({ status: 'checking', errorMessage: undefined });
    try {
      const url = await findLatestJsonUrl(channel);
      if (!url) {
        set({ status: 'idle' });
        return;
      }

      const metadata = await invoke<UpdateMetadata | null>('check_update_at_endpoint', { url });
      if (!metadata) {
        set({ status: 'idle' });
        return;
      }

      const update = new Update(metadata);
      set({
        status: 'available',
        pendingUpdate: update,
        currentVersion: update.currentVersion,
        latestVersion: update.version,
        changelog: update.body,
      });
    } catch (err) {
      set({ status: 'error', errorMessage: err instanceof Error ? err.message : String(err) });
    }
  },

  async downloadAndInstall() {
    const { pendingUpdate } = get();
    if (!pendingUpdate) return;

    set({ status: 'downloading', progress: { downloaded: 0 }, errorMessage: undefined });
    try {
      await pendingUpdate.downloadAndInstall((event: DownloadEvent) => {
        if (event.event === 'Started') {
          set({ progress: { downloaded: 0, total: event.data.contentLength } });
        } else if (event.event === 'Progress') {
          set((s) => ({
            progress: {
              downloaded: (s.progress?.downloaded ?? 0) + event.data.chunkLength,
              total: s.progress?.total,
            },
          }));
        }
      });
      set({ status: 'ready' });
    } catch (err) {
      set({ status: 'error', errorMessage: err instanceof Error ? err.message : String(err) });
    }
  },

  async relaunchNow() {
    await relaunch();
  },

  dismiss() {
    set({ status: 'idle', pendingUpdate: null, errorMessage: undefined, progress: undefined });
  },
}));
