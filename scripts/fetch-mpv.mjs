#!/usr/bin/env node
// Downloads the mpv binaries bundled with Apogee so the app never depends on
// a system-installed mpv. Windows has no package manager to depend on at
// all, so it's always bundled. Linux bundles it too (on top of also
// declaring `mpv` as a package dependency for .deb/.rpm - see
// src-tauri/tauri.conf.json's bundle.linux.deb/rpm.depends - as a harmless
// extra safety net) since CI builds deb/rpm/AppImage in a single pass and
// there's no per-format way to bundle only for AppImage without splitting
// that into a separate build invocation. macOS relies on the user installing
// mpv via Homebrew and gets a clear in-app error message if it's missing
// instead - see the plan this implements for why it isn't bundled there.
//
// Invoked automatically from tauri.conf.json's beforeBuildCommand via
// `node scripts/fetch-mpv.mjs auto`, which detects the host platform. Can
// also be run directly with an explicit target: `windows`, `linux`, or `all`.
//
// Windows: official shinchiro mpv build, distributed as a .7z archive via
// SourceForge (linked from https://mpv.io/installation/). Extracted with the
// `7zip-min` devDependency so no system 7z/7-Zip install is required.
//
// Linux: mpv has no official static/portable Linux build (see
// https://github.com/mpv-player/mpv/issues/4056). This uses the community
// "anylinux" build from pkgforge-dev/mpv-AppImage, which is itself a
// directly-runnable, dependency-free portable executable (built with
// "sharun", no FUSE/extraction required) - so it's used as-is as our bundled
// `mpv` binary, not unpacked.

import {
  createWriteStream,
  createReadStream,
  existsSync,
  mkdirSync,
  chmodSync,
  unlinkSync,
  renameSync,
  rmSync,
} from 'node:fs';
import { createHash } from 'node:crypto';
import { pipeline } from 'node:stream/promises';
import { Readable } from 'node:stream';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import sevenzip from '7zip-min';
// Node's built-in global fetch (a frozen vendored undici) hits a known
// assertion crash - "assert(!this.paused)" in Parser.finish - when a
// streamed download's destination write backpressures while the socket
// ends (nodejs/undici#5360, fixed in the standalone undici package at
// 8.4.1+). The fix hasn't landed in Node 24's bundled undici yet, so use
// the actively-maintained npm package here instead of the ambient global.
import { fetch } from 'undici';

const __dirname = dirname(fileURLToPath(import.meta.url));
const BINARIES_DIR = join(__dirname, '..', 'src-tauri', 'binaries');

const TARGETS = {
  windows: {
    url: 'https://sourceforge.net/projects/mpv-player-windows/files/release/mpv-0.41.0-x86_64.7z/download',
    sha256: 'ef86fde0959d789d77a3ad7c3c2dca51c6999695363f493a6154f2c518634c0f',
    outputName: 'mpv.exe',
    archiveEntry: 'mpv.exe',
  },
  linux: {
    url: 'https://github.com/pkgforge-dev/mpv-AppImage/releases/download/v0.41.0%402026-07-01_1782914175/mpv-v0.41.0-anylinux-x86_64.AppImage',
    sha256: '9ba489eb78c39fa4d5ef9cfaf9e80b92dcb9f69a05dd365d30255e6dca3c8fbd',
    outputName: 'mpv',
    // The pkgforge-dev "anylinux" build is produced with AppImage tooling and
    // ships two embedded ELF sections that AppImage desktop integrations
    // (AppImageLauncher, Gear Lever, appimaged, etc.) read to offer
    // self-updates: `.upd_info` (a zsync feed pointing at this project's
    // GitHub releases) and `.sig_key`. Since we bundle and version this
    // binary ourselves, that's exactly the "should mpv check for updates?"
    // popup users were seeing - strip both sections so the shipped binary
    // carries no update metadata at all.
    stripAppImageUpdateInfo: true,
  },
};

// macOS has no bundled target - it relies on a system mpv (Homebrew) and a
// clear in-app error message if it's missing, so `auto` is a no-op there.
const AUTO_TARGETS_BY_PLATFORM = {
  win32: ['windows'],
  linux: ['linux'],
  darwin: [],
};

async function download(url, destPath) {
  const res = await fetch(url, { redirect: 'follow' });
  if (!res.ok) throw new Error(`download failed: ${res.status} ${res.statusText} (${url})`);
  await pipeline(Readable.fromWeb(res.body), createWriteStream(destPath));
}

async function verifiedFetch(name, target) {
  const finalPath = join(BINARIES_DIR, target.outputName);
  if (existsSync(finalPath)) {
    console.log(`[fetch-mpv] ${name}: ${finalPath} already present, skipping`);
    return;
  }

  mkdirSync(BINARIES_DIR, { recursive: true });
  const downloadPath = join(BINARIES_DIR, `.download-${name}`);
  console.log(`[fetch-mpv] ${name}: downloading ${target.url}`);
  await download(target.url, downloadPath);

  const actualHash = await new Promise((resolve, reject) => {
    const hash = createHash('sha256');
    const stream = createReadStream(downloadPath);
    stream.on('data', (chunk) => hash.update(chunk));
    stream.on('end', () => resolve(hash.digest('hex')));
    stream.on('error', reject);
  });

  if (actualHash !== target.sha256) {
    unlinkSync(downloadPath);
    throw new Error(
      `[fetch-mpv] ${name}: checksum mismatch (expected ${target.sha256}, got ${actualHash}) - refusing to use this file`,
    );
  }

  if (target.archiveEntry) {
    // Windows build ships as a .7z archive - extract just the binary we need.
    const extractDir = join(BINARIES_DIR, `.extract-${name}`);
    mkdirSync(extractDir, { recursive: true });
    await new Promise((resolve, reject) => {
      sevenzip.unpack(downloadPath, extractDir, (err) => (err ? reject(err) : resolve()));
    });
    renameSync(join(extractDir, target.archiveEntry), finalPath);
    rmSync(extractDir, { recursive: true, force: true });
    unlinkSync(downloadPath);
  } else {
    // Linux build is a directly-runnable portable executable - use as-is.
    renameSync(downloadPath, finalPath);
  }

  if (target.stripAppImageUpdateInfo) {
    try {
      execFileSync('objcopy', ['--remove-section', '.upd_info', '--remove-section', '.sig_key', finalPath]);
      console.log(`[fetch-mpv] ${name}: stripped embedded AppImage update-info sections`);
    } catch (err) {
      throw new Error(
        `[fetch-mpv] ${name}: failed to strip AppImage update-info sections (is 'objcopy'/binutils installed?): ${err.message}`,
      );
    }
  }

  chmodSync(finalPath, 0o755);
  console.log(`[fetch-mpv] ${name}: wrote ${finalPath}`);
}

const requested = process.argv[2] ?? 'auto';

let names;
if (requested === 'auto') {
  names = AUTO_TARGETS_BY_PLATFORM[process.platform] ?? [];
  if (names.length === 0) {
    console.log(`[fetch-mpv] auto: no bundled mpv target for platform "${process.platform}", nothing to fetch`);
  }
} else if (requested === 'all') {
  names = Object.keys(TARGETS);
} else {
  names = [requested];
}

for (const name of names) {
  const target = TARGETS[name];
  if (!target) {
    console.error(`[fetch-mpv] unknown target "${name}" (expected one of: ${Object.keys(TARGETS).join(', ')}, auto, all)`);
    process.exit(1);
  }
  await verifiedFetch(name, target);
}
