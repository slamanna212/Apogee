import { create } from 'zustand';
import { Channel, invoke } from '@tauri-apps/api/core';
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
      // A large installer can emit hundreds of 'Progress' events per second;
      // committing a state update (and re-render) for every single one gives
      // the renderer no idle time to actually paint the bar's width, so the
      // number visibly climbs while the fill appears frozen. Track progress
      // locally and only flush to the store a few times a second.
      let downloaded = 0;
      let total: number | undefined;
      let lastFlush = 0;
      const flush = () => set({ progress: { downloaded, total } });

      // Routed through our own `download_and_install_update` command rather
      // than the plugin's `Update.downloadAndInstall()` - on Windows, the
      // plugin's own install step launches the installer in a way that gets
      // silently killed by Apogee's mpv-guarding Job Object the instant the
      // app exits afterward. See src-tauri/src/updater.rs for the fix; the
      // wire shape of these events matches the plugin's own DownloadEvent
      // exactly, so the handling below is unchanged.
      const onEvent = new Channel<DownloadEvent>();
      onEvent.onmessage = (event) => {
        if (event.event === 'Started') {
          downloaded = 0;
          total = event.data.contentLength;
          lastFlush = Date.now();
          flush();
        } else if (event.event === 'Progress') {
          downloaded += event.data.chunkLength;
          const now = Date.now();
          if (now - lastFlush >= 100) {
            lastFlush = now;
            flush();
          }
        } else if (event.event === 'Finished') {
          flush();
        }
      };
      await invoke('download_and_install_update', { rid: pendingUpdate.rid, onEvent });
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
