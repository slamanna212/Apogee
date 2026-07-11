# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Apogee is a Tauri (Rust + React/TypeScript) desktop app that acts as a radio tuner against an Xtream Codes IPTV backend, scoped to a single live-channel category (SiriusXM channels). It plays streams via a locally spawned `mpv` process and overlays now-playing metadata (song/artist) from the StellarTunerLog API by fuzzy-matching Xtream channel names to StellarTunerLog station names.

## Commands

- `npm run dev` — Vite dev server only (frontend, no Tauri shell)
- `npm run tauri dev` — full app in dev mode (spawns Tauri, which runs `npm run dev` for the frontend)
- `npm run build` — typecheck (`tsc -b`) + Vite production build of the frontend
- `npm run tauri build` — full native app bundle
- `npm run lint` — oxlint over the frontend
- `cargo build` / `cargo check` (run from `src-tauri/`) — build/check the Rust backend directly

There is no test suite (no test runner configured in `package.json`, no `#[test]` usage found) and no `npm test` script — don't assume one exists.

Requires the `mpv` binary on `PATH` at runtime; the Rust side spawns it as a subprocess and talks to it over a local IPC socket (`/tmp/apogee-mpv.sock` on Unix, a named pipe on Windows).

## Architecture

### Process split

- **`src-tauri/src/`** (Rust, backend/native layer):
  - `mpv.rs` — owns the `mpv` subprocess lifecycle. Lazily spawns `mpv --idle --input-ipc-server=...` on first `mpv_load` call, connects over a Unix socket/named pipe, and re-emits every line mpv writes to its IPC socket as a `mpv-event` Tauri event (the frontend does all further interpretation). Property reads (e.g. bitrate) are correlated back to the request via a fixed `request_id` (`777`, `GET_PROPERTY_REQUEST_ID`) rather than a request-tracking table, since only one property read is ever in flight.
  - `secrets.rs` — thin wrapper around the OS keyring (`keyring` crate) for storing the Xtream password and StellarTunerLog API key outside of the plaintext settings file.
  - `media_session.rs` — OS-level media session integration (`souvlaki`) so play/pause/toggle from OS media keys/notification comes back into the app as a `media-control-event`.
  - `lib.rs` — Tauri builder wiring: registers all `#[tauri::command]` handlers and the `http`/`store`/`log` plugins.
- **`src/`** (TypeScript/React, frontend):
  - `lib/mpvClient.ts`, `lib/secrets.ts` — direct `invoke()` wrappers around the Rust commands above; this is the only place that should call `invoke`/`listen` for those domains.
  - `lib/xtream.ts` — Xtream Codes `player_api.php` client (categories, live streams, stream URL construction).
  - `lib/stellarTunerLog.ts` — StellarTunerLog `/nowplaying` client.
  - `lib/channelMatcher.ts` — normalizes and fuzzy-matches (Levenshtein similarity, threshold `MATCH_THRESHOLD = 0.85`) Xtream channel names against StellarTunerLog station names, since the two systems don't share a stable ID for the same station.
  - `stores/` (Zustand) — one store per concern, each owning both state and the async actions that mutate it:
    - `settingsStore.ts` — persisted app settings via `@tauri-apps/plugin-store` (`settings.json`), with the Xtream password and StellarTunerLog API key kept out of that file and stored via `lib/secrets.ts` instead. Also migrates any plaintext secrets from older versions that stored them in the settings file directly.
    - `channelStore.ts` — fetches the channel list for the configured category and polls StellarTunerLog, producing the `streamId -> StellarStation` now-playing map via `channelMatcher`.
    - `playerStore.ts` — drives playback: builds the stream URL, calls `mpvClient`, and interprets the `mpv-event` stream to derive `status` (`idle`/`loading`/`playing`/`stopped`/`error`) and bitrate. Playback only flips to `playing` on mpv's `playback-restart` event (i.e. once buffering actually completes), not on the `loadfile` call succeeding. Also wires OS media-control events to `play()`/`stop()` (there's no pause for live radio — pause/toggle both map to stop).
  - `components/` — presentational React components consuming the stores above.

### Key flow

Settings (Xtream base URL/credentials + category, StellarTunerLog API key) → `channelStore.fetchChannels` loads the channel list → user selects a channel → `playerStore.selectChannel` builds a stream URL (`{baseUrl}/live/{user}/{pass}/{streamId}{extension}`) and loads it into mpv → mpv events flow back over the `mpv-event` Tauri event into `playerStore` → in parallel, `channelStore.pollNowPlaying` periodically fetches StellarTunerLog and fuzzy-matches it onto channels for display and OS media session metadata.

### Backend quirks (see `docs/milestone-0-findings.md`)

- The live Xtream backend this was built against serves raw MPEG-TS for both `.ts` and `.m3u8` requests (no real HLS) — `streamExtension` is a user setting, default `.ts`, and per-provider behavior shouldn't be assumed.
- Initial connection to a given `stream_id` occasionally times out on the first attempt (upstream channel spin-up); this is a known transient condition, not a hard failure.
