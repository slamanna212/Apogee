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
  tag_name: string;
  draft: boolean;
  prerelease: boolean;
  body: string | null;
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

export interface ChangelogEntry {
  version: string;
  body: string | null;
}

interface UpdateState {
  status: UpdateStatus;
  currentVersion?: string;
  latestVersion?: string;
  changelog: ChangelogEntry[];
  progress?: DownloadProgress;
  errorMessage?: string;
  pendingUpdate: Update | null;
  checkForUpdates: (channel: UpdateChannel) => Promise<void>;
  downloadAndInstall: () => Promise<void>;
  relaunchNow: () => Promise<void>;
  dismiss: () => void;
}

// Releases newer than the running version are shown in the changelog; this
// caps how many in case the running version's release was deleted (or is
// otherwise never found while walking the list), so an old install can't
// pull unbounded history.
const MAX_CHANGELOG_ENTRIES = 10;

// GitHub has no "latest release including prereleases" URL alias, so the
// channel is resolved here by walking the (newest-first, draft-free for
// unauthenticated requests) release list ourselves rather than relying on
// the static endpoint baked into tauri.conf.json.
async function fetchQualifyingReleases(channel: UpdateChannel): Promise<GithubRelease[]> {
  const response = await fetch(`https://api.github.com/repos/${REPO}/releases`, {
    headers: { Accept: 'application/vnd.github+json' },
  });
  if (!response.ok) {
    throw new Error(`GitHub API request failed: ${response.status}`);
  }
  const releases = (await response.json()) as GithubRelease[];
  return releases.filter((r) => !r.draft && (channel === 'beta' || !r.prerelease));
}

function findLatestJsonAsset(releases: GithubRelease[]): string | null {
  const asset = releases[0]?.assets.find((a) => a.name === 'latest.json');
  return asset?.browser_download_url ?? null;
}

// `latest.json`'s baked-in notes are frozen at CI draft-creation time, but
// release notes here are hand-written on GitHub afterward - so the live
// release list's `body` field (fetched above) is the source of truth, not
// the updater metadata's `body`. Releases are already newest-first, so this
// just walks forward collecting entries until it hits the running version.
function buildChangelog(releases: GithubRelease[], currentVersion: string): ChangelogEntry[] {
  const entries: ChangelogEntry[] = [];
  for (const release of releases) {
    if (entries.length >= MAX_CHANGELOG_ENTRIES) break;
    const version = release.tag_name.replace(/^v/, '');
    if (version === currentVersion) break;
    entries.push({ version, body: release.body });
  }
  return entries;
}

export const useUpdateStore = create<UpdateState>((set, get) => ({
  status: 'idle',
  pendingUpdate: null,
  changelog: [],

  async checkForUpdates(channel) {
    set({ status: 'checking', errorMessage: undefined });
    try {
      const releases = await fetchQualifyingReleases(channel);
      const url = findLatestJsonAsset(releases);
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
        changelog: buildChangelog(releases, update.currentVersion),
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
    set({ status: 'idle', pendingUpdate: null, errorMessage: undefined, progress: undefined, changelog: [] });
  },
}));
