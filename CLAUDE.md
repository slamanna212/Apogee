# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Apogee is a Tauri (Rust + React/TypeScript) desktop app that acts as a radio tuner against an Xtream Codes IPTV backend, scoped to a single live-channel category (SiriusXM channels). It decodes and plays streams in-process (Symphonia + CPAL) and overlays now-playing metadata (song/artist) from the StellarTunerLog API by fuzzy-matching Xtream channel names to StellarTunerLog station names.

## Commands

- `npm run dev` — Vite dev server only (frontend, no Tauri shell)
- `npm run tauri dev` — full app in dev mode (spawns Tauri, which runs `npm run dev` for the frontend)
- `npm run build` — typecheck (`tsc -b`) + Vite production build of the frontend
- `npm run tauri build` — full native app bundle
- `npm run lint` — oxlint over the frontend
- `npm test` — vitest run over the frontend (`src/**/*.test.ts`, colocated with the code under test)
- `cargo build` / `cargo check` (run from `src-tauri/`) — build/check the Rust backend directly
- `cargo test --workspace` (run from `src-tauri/`) — Rust tests across the app and `playback-core`

No external media player is required at runtime. Audio is decoded in-process and written to the
system audio device through CPAL.

**Build prerequisite on Linux:** ALSA development headers (`libasound2-dev` on Debian/Ubuntu,
`alsa-lib-devel` on Fedora). CPAL links ALSA dynamically, so *end users* need only the runtime
library, which every desktop already has; the `-dev` package is needed to compile.

`cargo` commands run from `src-tauri/`, which is a workspace containing the `apogee` app and the
`apogee-playback-core` crate. Core pipeline tests run without building Tauri:
`cargo test -p apogee-playback-core`.

## Architecture

### Process split

- **`src-tauri/src/`** (Rust, backend/native layer):
  - `network.rs` — the single HTTP layer. One `NetworkService` owning per-purpose Reqwest clients (JSON API, artwork, continuous TS, HLS playlist, HLS segment), each with its own policy. The continuous-TS profile deliberately has **no total request timeout** (a live stream runs for hours); it uses a connect deadline plus per-chunk stall detection. Credentials live in URL *path segments* here, so redaction strips the whole path and also sanitises Reqwest error strings, which embed URLs.
  - `playback/` — the engine. `source/` detects and reads (direct TS or HLS), `engine.rs` wires the stages, `audio_out.rs` owns the CPAL stream on its own thread (`cpal::Stream` is not `Send` everywhere; never "fix" that with an unsafe impl), `device.rs` enumerates and migrates devices, `commands.rs` is the only Tauri-facing part. Threading rule: network on Tokio, demux/decode/EQ/resample on a **dedicated OS thread**, and the audio callback only pops a lock-free ring.
  - `playback-core/` (separate crate, no Tauri dependency) — `detect.rs` (bounded content probe), `pipeline.rs` (the convergence point where both source paths meet as compressed access units), `decode.rs` (Symphonia), `dsp.rs` (10-band EQ + gain), `output.rs` (PCM ring, channel conversion, resampling), `session.rs` (generation-safe state machine), `analysis.rs` (spectrum FFT).
  - `secrets.rs` — thin wrapper around the OS keyring (`keyring` crate) for storing the Xtream password and StellarTunerLog API key outside of the plaintext settings file.
  - `media_session.rs` — OS-level media session integration (`souvlaki`) so play/pause/toggle from OS media keys/notification comes back into the app as a `media-control-event`.
  - `lib.rs` — Tauri builder wiring: registers all `#[tauri::command]` handlers and the `http`/`store`/`log` plugins.
- **`src/`** (TypeScript/React, frontend):
  - `lib/playerClient.ts`, `lib/secrets.ts` — direct `invoke()` wrappers around the Rust commands above; this is the only place that should call `invoke`/`listen` for those domains.
  - `lib/xtream.ts` — Xtream Codes `player_api.php` client (categories, live streams, stream URL construction).
  - `lib/stellarTunerLog.ts` — StellarTunerLog API client: `/nowplaying` and `/channels` (both keyless), `/history` (requires an API key, used only for per-channel play history in `ChannelModal`).
  - `lib/channelMatcher.ts` — matches Xtream channel names against StellarTunerLog station/channel names, since the two systems don't share a stable ID for the same station. Providers rename channels freely (number prefixes like `43 Rock The Bells Radio`, quality suffixes, reordered words), so each name is expanded into variants (as written, and with any channel-number prefix stripped by `parseChannelName`) and candidates are accepted on the best of four signals, in confidence order: exact normalized name, Levenshtein similarity ≥ `MATCH_THRESHOLD` (0.85), identical significant-word sets, then a channel-number hit corroborated by loose name agreement (≥ `NUMBER_CORROBORATION_THRESHOLD`). A bare number match is never accepted on its own — some providers prefix a group/EPG number rather than the SiriusXM channel number.
  - `lib/channelDisplay.ts` — what to call a channel in the UI: StellarTunerLog's marketing name and channel number when it matched, otherwise the provider's name cleaned up by `parseChannelName` (prefix/quality noise removed) and the number parsed out of it, rather than Xtream's lineup `num`. Unmatched channels are badged in the UI and listed in Settings → Diagnostics.
  - `stores/` (Zustand) — one store per concern, each owning both state and the async actions that mutate it:
    - `settingsStore.ts` — persisted app settings via `@tauri-apps/plugin-store` (`settings.json`), with the Xtream password and StellarTunerLog API key kept out of that file and stored via `lib/secrets.ts` instead. Also migrates any plaintext secrets from older versions that stored them in the settings file directly.
    - `channelStore.ts` — fetches the channel list for the configured category and polls StellarTunerLog, producing the `streamId -> StellarStation` now-playing map via `channelMatcher`.
    - `playerStore.ts` — a **projection of Rust state**, not a second state machine. It applies `player-snapshot` events, ignoring any whose `revision` is not newer than the last applied, and maps the Rust states onto the existing `status` union. URL construction, retries, the connect timeout and extension fallback all live in Rust now; do not reintroduce them here. Playback flips to `playing` only when the audio callback has actually consumed decoded frames, not when a request succeeds. OS media-control events map to `play()`/`stop()` (there's no pause for live radio).
  - `components/` — presentational React components consuming the stores above.

### Key flow

Settings (Xtream base URL/credentials + category; an optional StellarTunerLog API key needed only for per-channel play history) → `channelStore.fetchChannels` loads the channel list → user selects a channel → `playerStore` sends a typed `StationRequest` to `player_play` → **Rust** builds the stream URL (`{baseUrl}/live/{user}/{pass}/{streamId}{extension}`, alternating `.ts` and `.m3u8` across attempts), detects whether the body is MPEG-TS or an HLS playlist from its bytes rather than its extension, demuxes, decodes and plays it → state flows back as `player-snapshot` events → in parallel, `channelStore.pollNowPlaying` periodically fetches StellarTunerLog and fuzzy-matches it onto channels for display and OS media session metadata.

### Backend quirks (see `docs/milestone-0-findings.md`)

- **Never route on the file extension.** Historically this backend served raw MPEG-TS from both `.ts` and `.m3u8` URLs. Re-measured 2026-09-10, it now serves genuine HLS from `.m3u8` and raw TS from `.ts`. Since neither behaviour can be assumed, the content is identified from its first bytes (`playback-core/src/detect.rs`) and the controller alternates `.ts`/`.m3u8` across its bounded retry attempts.
- Both endpoints **302-redirect off the original host**, and the redirect itself is served as `text/html`. HLS segment URIs are absolute paths, so they must resolve against the *post-redirect* URL or requests go to the wrong host. The final hop is plain `http`, so do not force a scheme upgrade.
- `hls-runtime` has two confirmed gaps: it cannot parse a master playlist (it errors and then goes permanently dead), and it has no encryption support at all. Master playlists are handled by selecting a variant before handing off; `EXT-X-KEY` is detected and refused with a clear error.
- Symphonia decodes **AAC-LC only**. Other AAC object types return an unsupported error rather than degrading.
- Initial connection to a given `stream_id` occasionally times out on the first attempt (upstream channel spin-up); this is a known transient condition, not a hard failure.
