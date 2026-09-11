# Symphonia migration progress

## Current status

M1-M5 are implemented in the working tree. The Rust engine is the production playback path and MPV has been removed. Linux audible playback was confirmed on 2026-09-10; the corrective review in `rustaudiofixes.md` was implemented on 2026-09-11. Windows/macOS execution, device hotplug/default-following hardware checks, and a controlled updater install remain manual release gates.

| Milestone | Status | Evidence / remaining work |
| --- | --- | --- |
| M0: dependencies, baseline, fixtures | Complete | Crate builds; 4 tests pass, incl. direct-TS vs HLS bit-exact PCM parity on real provider audio |
| M1: networking | Complete | NetworkService plus the full API migration; plugin-http removed |
| M2: source-to-PCM | Complete | Detection, shared pipeline, both ingest adapters, networked TS and HLS sources; 84 tests |
| M3: output/controller | Complete (automated + prior Linux audible check) | Callback gates consumption; EQ/volume affect queued PCM; post-control visualizer uses a bounded worker; rate/channel changes rebuild conversion |
| M4: active devices | Complete (automated); hardware matrix pending | Active selection restarts safely under a new generation; system default is followed by bounded polling; requested and effective devices remain separate |
| M5: AppImage signing | Complete (code); CI/manual artifact run pending | AppImage is finalized before signing; pinned repack tools; serialized manifest updates; no post-sign mutation |
| M6: integrated acceptance | Partial | 195 Rust and 187 frontend tests pass; builds/checks pass; native Windows/macOS and remaining hardware/artifact checks pending |

## Work log

- Read plan and current playback/network/settings/update integration. Preserved existing worktree edits.
- User checked five provider stations in Media Player Classic: AAC-LC, 44.1 kHz stereo, approximately 258 kbps. This is user-reported codec evidence; no provider capture or live playback test has been performed by this implementation.
- Shell PATH omitted installed Node/Rust. Located Node 24.13.1 and Rust 1.97.0; use explicit PATH additions in this environment.
- Baseline: `npm test` passed 181 tests in 17 files. Lint passed with two existing ChannelCard fast-refresh warnings. Frontend build passed with an existing bundle-size warning.
- Baseline Rust check started with `--locked --offline`; result pending.
- crates.io metadata API returned HTTP 403 both sandboxed and escalated. Official static package downloads work. Downloaded hls-runtime 0.6.0, transmux 0.24.0, and container-probe 0.1.0 source archives to `/tmp/apogee-playback-deps` for inspection.
- Confirmed hls-runtime emits compressed `Output::Samples`, preceded by `Output::Init` containing MP4 initialization bytes (synthesized even for TS). The decoder bridge must parse initialization metadata once, not demux sample payloads again.
- hls-runtime 0.6.0 requires Rust 1.95; local Rust 1.97 is sufficient, but repository MSRV/CI must be reconciled before integration.

- Toolchain repair: `~/.cargo/bin` had been deleted during a disk cleanup, removing the `rustup` binary, every shim, and `~/.cargo/env` (sourced by both `.profile` and `.bashrc`); `~/.rustup/settings.toml` was empty so no default toolchain was set. This, not a PATH quirk, was the cause of the earlier "shell PATH omitted Node/Rust" note. Reinstalled rustup with `--no-modify-path`; stable is now 1.98.1. The package registry survived intact.
- The earlier crates.io HTTP 403 was a symptom of the broken install, not a network restriction. With the toolchain repaired, all dependencies resolve normally from the standard registry. The `/tmp/apogee-playback-cargo` CARGO_HOME workaround and the `/tmp/apogee-playback-deps` hand-downloaded archives were deleted; the crate builds against the default cargo home.
- `src-tauri/.gitignore` ignored `/target/`, which is anchored and so never matched the nested `playback-core/target`. 253 MB of build artifacts (566 files) were showing as untracked. Rule changed to `target/`.
- Root-caused the failing parity test. It was a bad fixture, not a pipeline defect. The original `aac-*.ts` set was a continuous capture chopped at fixed byte offsets that ended mid-frame: `aac-4.ts` held a single 188-byte audio packet, roughly 184 bytes of payload, where one AAC frame at 258 kbps needs about 748 bytes. Decoded standalone that segment yielded no frames; concatenated it completed a frame `aac-3.ts` left dangling. That accounted for the entire 2048-sample (one stereo AAC frame) discrepancy. The HLS path was correct throughout.
- Regenerated fixtures from a live provider station: probed as AAC-LC, 44100 Hz, stereo, 258434 bps, which independently confirms the user's Media Player Classic reading. Captured 12 s, took a 3 s slice, and segmented it with ffmpeg's HLS muxer so cuts land on frame boundaries. Six segments, about 107 KB. Per-segment sum, concatenated stream, and HLS client output now all yield 130 frames / 266240 samples.
- `cargo test` in `playback-core`: 4 passed, 0 failed, stable across three consecutive runs. `cargo clippy --all-targets` clean. Main app `cargo check` builds and `cargo test` passes 8 tests after the toolchain repair.
- Capture credentials are deliberately not recorded here. Fixtures are committed audio only.

## Scope decisions (2026-09-10)

- Execute the full plan through M5, including MPV removal. Reverting is handled through git.
- M1 was initially reduced to **media networking only**, then completed during M5 on the user's
  instruction once audible playback was confirmed. The single-HTTP-stack goal is now met.
- Nothing was committed by the agent. The user committed M1-M4 themselves as
  `start work on rust playback engine`; the M5 work sits uncommitted on top for their review.
- Physical audio validation is Linux-only. See `symphonia-platform-acceptance.md`.
- `src-tauri` is now a Cargo workspace with `playback-core` as a member, so core pipeline tests run without building Tauri. MSRV raised from 1.77.2 to 1.95 to match hls-runtime; CI already used `dtolnay/rust-toolchain@stable`, so the previous 1.77.2 declaration was already inaccurate.

## M2 progress (headless pipeline)

`playback-core` now carries three modules and 22 passing tests, clippy and rustfmt clean.

- `detect.rs` — bounded prefix detection over container-probe. Content decides, never the URL
  extension or MIME type, because the provider serves raw MPEG-TS from `.m3u8` with an HLS content
  type. Retains every probed byte for replay so detection is non-destructive. Distinguishes
  `NeedMoreData` from a definitive `Unsupported`, and reports budget exhaustion rather than asking
  forever. HTML and JSON bodies served as HTTP 200 become actionable errors instead of endless
  decoder probing. `EXT-X-KEY` is rejected up front per the owner decision. Master playlists are
  identified and `select_variant` picks a rendition deterministically by highest `BANDWIDTH`, with
  URI tie-breaking and correct handling of commas inside quoted `CODECS` attributes.
- `pipeline.rs` — the convergence point. Both paths already speak `transmux::{TrackSpec, Sample}`,
  so that pair is used directly as the shared representation; inventing a parallel struct would add
  a translation layer that could silently diverge, which is the exact failure the plan warns about.
  `TsIngest` wraps `StreamingTsDemux`; `HlsIngest` parses only `Output::Init` metadata and forwards
  `Output::Samples` untouched. One decoder per session, reset only on a real discontinuity.
- Bitrate is computed from audio access units only, never transport throughput, and stays `None`
  until at least a second of audio has decoded.

The parity test now asserts more than sample equality: exactly one initialization section, zero
decoder resets on both paths, and equal frame counts. A separate test proves the reported bitrate
lands in the AAC range rather than the inflated transport figure, and that the audio payload is
smaller than the TS carrying it.

## Live provider re-check (2026-09-10) — corrects a plan premise

The plan and `docs/milestone-0-findings.md` state that the provider returns raw MPEG-TS for **both**
`.ts` and `.m3u8`. That is **no longer true**. Measured directly against a live station:

| Endpoint | Redirects | Final content type | Body |
| --- | --- | --- | --- |
| `.ts` | 1 (302) | `video/mp2t` | Raw MPEG-TS, 188-byte stride, sync `0x47` |
| `.m3u8` | 1 (302) | `application/x-mpegURL` | A genuine live HLS media playlist |

The playlist is a real sliding-window live playlist: `EXT-X-VERSION:3`, `EXT-X-MEDIA-SEQUENCE:849`,
`EXT-X-TARGETDURATION:11`, roughly 10-second segments, no `EXT-X-ENDLIST`, no `EXT-X-KEY`, and no
`EXT-X-STREAM-INF`. So it is a media playlist, not a master, and it is unencrypted.

Consequences for M2, all load-bearing:

- **HLS is genuinely exercisable against this provider.** The plan's warning not to mark HLS
  implemented on the strength of `.m3u8` URLs that actually contain TS does not apply here, but the
  local-fixture HLS tests stay regardless.
- **Both endpoints 302 before serving content, and the redirect leaves the original host.** The
  intermediate 302 is served as `text/html`, so content type must never be trusted before the final
  response.
- **Segment URIs are absolute paths** of the form `/hls/<hash>/<id>_<seq>.ts`. They must be resolved
  against the *effective post-redirect* URL. Resolving against the originally requested URL would
  produce requests to the wrong host. This is the single most likely way to get a silently broken
  HLS path, and it is now a known-live condition rather than a hypothetical.
- The final hop is plain `http`, not `https`, so the scheme restriction must permit http and the
  redirect policy must not force a scheme upgrade.

Verified: the detector identifies the real `.ts` body as `MpegTs { stride: 188 }` and the real
`.m3u8` body as `HlsMediaPlaylist`, and still returns `MpegTs` for the TS body when handed a
deliberately wrong `text/html` content type.

## DSP finding: MPV's headroom rule under-reserves and clips

`playback-core/src/dsp.rs` implements the ten-band EQ plus smoothed gain, 13 tests, all
measuring real frequency response rather than asserting the code ran.

Building it surfaced a defect in the **existing** app, not just the migration. MPV was handed
`volume=-{max_boost}dB`, reserving headroom for the single largest band boost. That is not enough:
adjacent peaking filters overlap, so their magnitude responses multiply. Measured here, ten bands at
+12 dB peak at **2.17** after MPV's headroom is applied, i.e. roughly 6.7 dB past full scale. The
current MPV-based build will clip on that setting.

Rather than reproduce the flaw, `configure` now measures the true worst-case magnitude of the whole
filter cascade on a 1024-point log-spaced grid and reserves exactly its inverse. Both behaviours are
covered by tests: for a single boosted band the measured headroom matches MPV's rule to within
0.01 at 3, 6 and 12 dB, so ordinary settings are unchanged; for overlapping boosts strictly more
headroom is reserved and a full-scale input no longer exceeds 1.0. The grid evaluation happens in
`configure`, never in the audio callback.

Other DSP properties now covered: a disabled EQ is bit-exact unity; a flat enabled EQ shifts level by
under 0.01 dB; per-channel filter state is independent, proven by a silent channel staying silent;
gain changes ramp so they cannot click; mute preserves the stored volume; band centres at or above
Nyquist are skipped instead of producing non-finite output at low device rates; out-of-range and
non-finite gains are rejected rather than silently clamped; and 20 seconds of continuous processing
with alternating extreme gains stays finite and bounded.

## Behaviour to preserve (read from current source, for M3/M4)

Retry/connect defaults in `src/stores/playerStore.ts`, to be moved into the Rust controller:
`MAX_CONNECT_ATTEMPTS = 4`, `RETRY_DELAY_MS = 1500`, `CONNECT_TIMEOUT_MS = 20000`, alternating
`.ts` then `.m3u8` per attempt. Frontend `PlayerStatus` is `idle | loading | playing | stopped | error`
with a separate `isBuffering` flag; the Rust states Stopped/Connecting/Buffering/Playing/Recovering/
Failed must project onto that shape.

EQ parity target, read from `build_equalizer_filter` in `src-tauri/src/mpv.rs`: ten bands at
31/62/125/250/500/1k/2k/4k/8k/16k Hz emitted as ffmpeg `equalizer=f={freq}:t=o:w=1:g={gain}`. That is
a **peaking (bell) filter with a one-octave bandwidth**, so the equivalent biquad Q is
`sqrt(2^1)/(2^1 - 1) = 1.414`. When any gain is positive the filter chain is prefixed with
`volume=-{max_boost}dB`, i.e. automatic headroom equal to the largest boost, which is why boosting
does not clip. The CPAL DSP must reproduce both the per-band shape and that pre-gain, and gains stay
clamped to -12..+12 dB. Bit-exact parity with libavfilter is not claimed.

## Visualizer contract to preserve (M4)

From `src-tauri/src/waveform.rs`, so the UI keeps working when the source changes from system-audio
capture to a direct post-EQ PCM tap:

- Event name `waveform-levels`, payload a plain `Vec<f32>` of **8** band levels (9 edges in
  `BAND_EDGES_HZ`). `src/components/Waveform.tsx` consumes this shape and should not need changing.
- 1024-point FFT window. Asymmetric smoothing: 0.05 s attack, 0.3 s release, applied to the displayed
  level like a VU meter needle. Display range floor -70 dB, ceiling -35 dB. A slow 8-second volume
  adaptation normalises toward -40 dB so quiet and loud stations look similar.
- `BAND_TILT_COMPENSATION_DB` (8 values, -18 to +22 dB) is **capture-specific** and must be
  re-evaluated, not copied. It compensates for the response of the loopback/parec capture chain. A
  direct PCM tap has no such colouration, so carrying these values over unexamined would visibly
  skew the spectrum. The plan calls this out explicitly.
- The direct tap is taken post-EQ and post-volume, so it reflects what Apogee outputs before OS or
  hardware effects. Levels are therefore pre-OS-volume, unlike capture, which means the fixed floor
  and ceiling may need retuning even though the adaptive normalisation should absorb most of it.
- Removing capture means the app no longer needs microphone or system-audio permission for the
  spectrum, and the macOS entitlement and its usage description can go with it.

## M5 removal inventory (measured, not assumed)

336 case-insensitive `mpv` references across 27 non-doc files. Counts:

| Refs | File | In plan section 12? |
| --- | --- | --- |
| 100 | `src-tauri/src/mpv.rs` | yes (delete) |
| 43 | `src/stores/playerStore.test.ts` | **no** |
| 32 | `src/stores/playerStore.ts` | yes |
| 26 | `scripts/fetch-mpv.mjs` | yes (delete) |
| 16 | `src/lib/mpvClient.ts` | yes (replace) |
| 15 | `src-tauri/src/lib.rs` | yes |
| 7 | `src/pages/Settings.tsx` | yes |
| 6 | `CLAUDE.md` | yes |
| 5 | `src-tauri/src/waveform.rs` | yes |
| 5 | `src-tauri/src/updater.rs` | yes |
| 3 | `.vscode/tasks.json` | **no** |
| 3 | `src-tauri/tauri.conf.json` | yes |
| 2 | `src-tauri/src/waveform/macos_capture.rs` | yes |
| 2 | `src-tauri/src/logs.rs` | **no** |
| 2 | `src-tauri/src/discord_rpc.rs` | **no** |
| 2 | `src-tauri/entitlements.plist` | yes |
| 2 | `src/lib/waveform.ts` | **no** |
| 2 | `.github/ISSUE_TEMPLATE/bug_report.md` | **no** |
| 1 each | `tauri.windows.conf.json`, `tauri.linux.conf.json`, `waveform/windows_capture.rs`, `src-tauri/.gitignore`, `src/stores/updateStore.ts`, `src/stores/settingsStore.ts`, `src/main.tsx`, `src/components/Waveform.tsx`, `package.json` | partly |

Eight files carrying MPV references are **not** named in the plan's cleanup list. The largest by far
is `playerStore.test.ts` at 43 references: the existing playback regression tests are written against
MPV events, so M4 must rewrite rather than delete them. The plan warns against replacing real
regression tests with mocks that merely return success, so these need genuine replacements driven by
the new controller's events.

`README.md` contains no MPV references despite being listed in the plan, so nothing to do there.

## M3 controller core

`playback-core/src/session.rs`, 15 tests, no I/O or time source so the races are tested directly
rather than provoked through a live pipeline.

Every accepted play or stop takes a strictly increasing generation, so an asynchronous completion
belonging to an abandoned session is rejected instead of emitting audio or overwriting state.
Selecting the same station twice deliberately yields two generations, since channel identity alone
cannot separate them. A stopped controller accepts nothing, including for its own current generation.

Snapshots carry a monotonic revision that advances on every observable change, so a late snapshot is
identifiable as stale rather than overwriting a newer event.

Retry behaviour preserves the existing defaults: four attempts, 1.5 s delay, 20 s connect timeout,
alternating `.ts`/`.m3u8`. Permanent errors such as bad credentials skip retrying entirely and do not
consume the budget. The budget refills only after 30 seconds of uninterrupted audible playback, never
because a request succeeded; a test drives a stream that connects, plays briefly and drops twenty
times over and asserts it still terminates.

`PlaybackState` projects onto the existing frontend union in `src/types/player.ts` so the UI contract
is unchanged, and exposes `is_audible` separately so scrobbling and Discord presence credit only real
playback rather than a successful connection.

## Build prerequisite discovered: ALSA development headers

Promoting CPAL from Windows-only to all desktop targets fails to compile on this Linux box:

```
HINT: if you have installed the library, try setting PKG_CONFIG_PATH to the directory
containing `alsa.pc`.
```

`libasound2t64` (the runtime shared library) is installed and `/usr/lib/x86_64-linux-gnu/libasound.so.2`
exists, but `libasound2-dev` is not, so there are no headers and no `alsa.pc`. `pkg-config --exists alsa`
fails. Fix is `sudo apt install libasound2-dev`, which needs the user.

**Build-time only, confirmed from source.** `alsa-sys-0.4.0`'s build script calls
`pkg_config::Config::new().statik(false).probe("alsa")`. `statik(false)` means it links the shared
library dynamically, and the crate declares `links = "alsa"`. So:

| Who | Needs | Why |
| --- | --- | --- |
| This dev box | `libasound2-dev` | headers + `alsa.pc` to compile |
| CI Linux runner (`rapture-apogee`) | `libasound2-dev` | same, once CPAL is promoted |
| End users | `libasound2` runtime only | dynamic link at load time |

End users do **not** need the `-dev` package. They need the ALSA runtime shared library, which is
already present on essentially every Linux desktop (PipeWire and PulseAudio both pull it in) and is
already installed on this machine as a transitive dependency.

Packaging consequence for M5: `tauri.conf.json` currently declares `deb.depends` and `rpm.depends` as
`["mpv"]`. When MPV is removed, that should become the ALSA runtime library rather than nothing, so
the package manager can guarantee it. This is a **net reduction** in end-user install burden: mpv is
a large dependency, `libasound2` is a small one that is already universally present.

To keep the workspace building meanwhile, CPAL stays Windows-only and the device-independent parts of
M3 (PCM ring, channel conversion, resampling) were placed in `playback-core`, which needs no audio
device. Only the CPAL device layer is blocked.

## M2 networked sources: verified

`src-tauri/src/playback/source/` drives both paths over the shared `NetworkService`. Nine tests,
covering: a `.m3u8` URL whose body is raw MPEG-TS routing to the direct path; probed bytes replayed
intact rather than refetched; an HTML 200 body becoming an actionable error; credential redaction;
a redirect chain followed with the final URL used downstream; a live playlist with no `ENDLIST`
continuing to refresh; and cancellation returning promptly both mid-playlist-wait and mid-segment-fetch.

The decisive one uses two real servers and asserts **zero** segment requests reach the originally
requested host while access units actually decode, which is the failure mode the live provider's
absolute-path segment URIs would otherwise cause.

Review correction: `slice_byte_range` carried a comment claiming it was covered by a synthetic test,
and no such test existed. The implementation was in fact sound, using checked arithmetic and a
bounds-checked slice, but it was unverified. Four tests were added covering valid ranges, zero length,
ranges past the end, and arithmetic overflow, and the comment was corrected. Byte ranges are fetched
whole and sliced locally because `NetworkService` has no HTTP Range path; correct but not
bandwidth-optimal, and unreachable for this provider's playlists.

## M3 output layer: verified on real hardware

`playback-core/src/output.rs` (device-independent) plus `src-tauri/src/playback/{audio_out,device}.rs`
(CPAL). 115 tests total across the workspace, four pre-existing clippy warnings and no new ones.

**Ring, conversion, resampling** (`output.rs`): a bounded frame-addressed PCM ring on `rtrb`, explicit
mono/stereo/N-to-M channel rules, rubato resampling with a genuinely zero-copy bypass at equal rates,
and a `BufferGate` with separated start/rebuffer thresholds so state cannot oscillate.

Review addition: `pop_into` reports whole frames by dividing samples by channel count, which is only
correct while the ring's contents stay frame-aligned. That invariant holds because capacity is
allocated as `frames * channels`, so free space is always frame-aligned and a push cannot stop between
the channels of one frame. It was undefended, and a change to how capacity is computed would have
produced a permanent channel swap rather than an obvious failure. Three tests now pin it, including
500 rounds of adversarially mismatched push and pop sizes asserting every frame stays a consecutive
pair.

**Devices** (`device.rs`): enumeration, descriptors carrying a persistable id, and resolution that
falls back to the system default when a specific device is gone while reporting what was actually
opened, so a temporary unplug does not erase the stored preference. Legacy MPV identifiers are
migrated by exact id match, then by unique descriptive match; an ambiguous or unmatched value falls
back to the default **with an explanation**, never to silence. Nine tests, three against real hardware.

**CPAL owner** (`audio_out.rs`): `cpal::Stream` is not `Send` on every platform, so it is created,
held and dropped on one dedicated thread, with commands by channel. No unsafe `Send`/`Sync` impls, as
the plan requires. The callback only pops from the lock-free ring and publishes counters through
atomics; the per-format scratch buffer is allocated once at stream construction, never in the callback.
Playback confirmation comes from the callback having consumed a real frame, not from an HTTP success.
Output failure is tracked separately from network starvation.

Three CPAL tests run against real hardware, deliberately selecting a null sink so the suite makes no
noise: the callback consumes queued audio and confirms playback, a dry ring underruns into silence
without being reported as an output failure, and dropping the output stops and joins its thread
promptly rather than hanging.

Two further CPAL 0.18 API differences found by compiling rather than assuming: the error callback
takes `cpal::Error` (not a separate `StreamError` type), and `build_output_stream` takes its config by
value.

## Historical M3/M4 checkpoint: 127 tests

`src/playback/` now holds `engine.rs` (session wiring), `audio_out.rs` (CPAL owner),
`device.rs` (enumeration and migration) and `commands.rs` (Tauri adapter). Typed commands
replace MPV's arbitrary property strings: play, stop, volume, mute, equalizer, list/set/migrate
device, visualiser toggle, and snapshot.

Threading follows the plan's ownership rules. Network and HLS timers run as a Tokio task; demux,
decode and resampling run on a **dedicated OS thread**, never on a Tokio worker. The CPAL callback
gates ring consumption and applies precomputed EQ coefficients plus ramped gain without allocation,
locking or I/O. Post-control samples enter a bounded nonblocking tap consumed by a separate FFT
worker. Teardown order matters and is deliberate: cancel the token, drop
the event sender to wake a parked decode thread, drop the output to join the audio thread, then join
decode. That is what makes a station switch safe rather than racy.

Three end-to-end tests run the whole chain against a real device: a fixture served as chunked
MPEG-TS over HTTP decodes through detection, demux, decode, EQ, resampling and the ring until the
audio callback confirms playback. They assert the reported format is the **stream's** 44100 Hz rather
than the device's 48000, that bitrate lands in the AAC range rather than reporting transport
throughput, that teardown completes in well under a second, and that an HTML error body fails the
session instead of hanging.

MPV and its capture path have been removed from production.

### Corrective review completed 2026-09-11

The original M3 implementation announced buffering but did not make the callback obey it, and it
processed volume/EQ/FFT before the two-second PCM queue. The callback now emits silence without
draining during startup/rebuffering, applies ready-made controls to the next consumed frames, and
reports every buffering/playing transition through atomics. The FFT worker can fall behind only by
dropping analysis samples; it cannot block audio. Conversion tracks `(sample_rate, channels)`, so
mono/stereo changes at a stable rate rebuild state and discard pending old-format resampler input.

Each retry now also has an internal attempt identity in addition to its station generation. Starting
a retry invalidates the previous identity before teardown, preventing a late watchdog/output event
from scheduling a duplicate retry. Active device changes deliberately reconnect the station under a
new generation after closing the old session. CPAL has no portable default-device notification API,
so system-default mode polls every two seconds outside the callback and reopens only when the
effective default id changes. A missing specific device falls back without erasing the saved choice.
The healthy fallback stream does not automatically jump back when that specific device reappears;
the user must select it again, or a later session/retry will resolve the preserved preference.

The 2026-09-11 local unsigned AppImage validation reached a successful optimized application build,
then Tauri timed out downloading its `AppRun`/`linuxdeploy` helpers before an AppImage was produced.
The packaging scripts and workflow YAML pass local syntax checks, and the pinned tool/runtime hashes
were verified, but final extraction/signature/updater verification remains an artifact-run gate.

## Visualiser: recalibrated, not copied

`playback-core/src/analysis.rs` taps post-EQ, post-volume PCM. The event name `waveform-levels` and
its 8-band `Vec<f32>` payload are unchanged, so `Waveform.tsx` needs no modification. No microphone
or system-audio permission is involved.

The plan warned that the capture-specific compensation must be re-evaluated rather than carried over.
Splitting it turned out to matter:

- `BAND_TILT_COMPENSATION_DB` is **kept**. Its own comment attributes it to the spectral tilt of real
  music and to log-spaced bands averaging over very different numbers of linear FFT bins. Neither
  depends on how the audio was obtained.
- `LEVEL_FLOOR_DB`/`LEVEL_CEILING_DB` are **recalibrated**. The old -70..-35 dB was measured against
  parec output downstream of system volume; this tap sits upstream of the OS mixer. Measured against
  decoded provider audio over 130 windows, per-band averages ran -50.9 to -69.6 dB, overall -61.4 dB.
  The old window would have pegged every bar. The new -85..-40 dB places that average at 0.52 of full
  scale.

The calibration comes from a short sample of one station, so it is a starting point that wants
checking by eye across several stations. A test asserts the average maps to between 0.3 and 0.7 of
full scale and that bands retain spread, so drifting into a wall of full or dark bars fails rather
than shipping. Digital silence reads as zero without producing non-finite values, levels release
gradually rather than snapping, and a low-rate device does not produce garbage in bands above Nyquist.

Analysis is skipped entirely when the visualiser is off, so a hidden display costs nothing.

## Historical M4 checkpoint: the app now plays through the new engine

316 tests pass (127 Rust, 189 frontend). Lint shows only the two pre-existing ChannelCard warnings,
`npm run build` and `cargo build` are clean, and three pre-existing clippy warnings remain in files
this migration has not touched.

- `src/lib/playerClient.ts` is the only place the frontend invokes the player. `playerStore.ts` is now
  a projection of Rust state: the frontend retry loop, connect timeout, extension alternation and
  mpv-event parsing are gone. Nothing in `src/` imports `mpvClient.ts` any more.
- Snapshots are applied only when `revision` exceeds the last applied one, so an event arriving before
  its `invoke` resolves cannot be overwritten by a stale reply.
- The visualiser is switched to the in-process PCM tap. `waveform::ensure_started` is no longer called,
  so **no system-audio capture process is spawned at all**. The old module is retained behind
  `#[allow(dead_code)]` purely so the change can be reverted in one step.

### Two integration defects found in review, not by tests

1. **A successful device migration silently undid itself.** `player_migrate_device` adopted the matched
   CPAL device in the engine but returned only an explanation, so the frontend persisted `null`. Engine
   and saved settings disagreed, and the next launch reverted to the system default. The command now
   returns `{ deviceId, notice }` and the frontend persists the id. A regression test covers the
   successful path, which previously had no coverage at all.
2. **Both visualiser paths emitted the same event.** The frontend still called `waveform_set_active`,
   so the old capture path stayed live while the new tap sat dormant behind a flag nothing set.

### Also fixed

`oxlint` was failing on seven files: the MPEG-TS fixtures use a `.ts` extension, so it read binary
transport-stream data as TypeScript. `src-tauri/**` is now excluded from the frontend linter.

### Deliberate behaviour changes, recorded rather than hidden

Volume, mute, EQ and device selection are pushed once at startup and on user change, not reapplied on
every connect. The MPV subprocess reset its state per `loadfile`; the CPAL output stage is long-lived
and does not. Volume and mute also now apply with no station selected, since they are properties of
the output stage rather than of a playback session.

At this checkpoint MPV was still registered as a rollback path. M5 subsequently removed it; the
current state is described below. See `symphonia-manual-test-guide.md`.

## M5 complete (code)

### MPV removed

`mpv.rs`, `waveform.rs`, the two platform capture modules, `mpvClient.ts` and
`scripts/fetch-mpv.mjs` are deleted. The Windows Job Object is gone, and with it the
`win32job` dependency. `rustfft` moved to `playback-core` with the analyser and `libc` went
with the Linux capture path.

Packaging: `beforeBuildCommand` no longer fetches MPV, the platform overlays bundle no
resources, and the Linux package dependency changed from `mpv` to the ALSA runtime library.
That is a **net reduction** in what users must install. The macOS entitlements comment no
longer cites Homebrew MPV discovery and records that no capture or microphone entitlement is
needed. `libasound2-dev` was added to the CI Linux dependency step.

The updater's custom Windows installer path was **kept** while its job-object coupling was
removed. The plan asks to preserve installer launch-failure reporting, and that is a separate
concern from the breakaway flag: the updater plugin ignores the installer launch's return
value and then exits unconditionally, so a failed launch would otherwise leave the user with
no installer, no update and no error.

### Networking consolidated

Xtream (`xtream.rs`), StellarTunerLog (`stellar.rs`), the GitHub release list (`updater.rs`),
Last.fm and notification artwork all now go through `NetworkService`. `tauri-plugin-http` and
its wildcard `http:default` capability are removed, as is `fetchWithTimeout.ts`.

Credential handling was the reason to move Xtream in particular: its credentials travel in
**query parameters**, and a transport error's `Display` embeds the URL. Those messages are
surfaced directly in UI-visible store state, so a leak would have been user-visible. Tests
assert no user-facing error contains a URL or a credential.

**One HTTP stack, with two documented exceptions**, both framework-owned rather than
application-owned: channel artwork loaded by `<img>` tags in the webview, and the updater
plugin's own download of a release artifact. The plan permits these provided they are
explicit, which they now are in `network.rs`'s module documentation.

### Verification

141 Rust tests and 186 frontend tests pass. `cargo clippy --workspace --all-targets` reports
zero warnings; `npm run lint` reports only the two pre-existing ChannelCard warnings. Both
builds are clean.

The frontend test count fell from 189 because three `buildStreamUrl` tests were deleted along
with the function; Rust now builds stream URLs and carries equivalent tests.

## Completion checklist (plan section 14)

| Item | Status |
| --- | --- |
| Both source paths use the same HTTP service and converge at compressed access units, one demux stage each | Met |
| Direct TS and hls-runtime share compatible transmux; `mpeg2ts-reader` is not a parallel demuxer | Met — `mpeg2ts-reader` appears nowhere in `Cargo.lock`, and `cargo tree -d` shows no duplicate transmux |
| hls-runtime actually integrated and tested with real playlist fixtures | Met |
| No extension-only routing assumption remains | Met — routing is by content probe; a `.m3u8` serving raw TS is covered by a test |
| MP3/AAC scope verified against documented profile/framing fixtures | Met, with the documented AAC-LC-only limit |
| One controller owns retries, cancellation and playback truth | Met |
| Bounded buffering and session generations tested under adversarial timing | Met |
| Devices, EQ, mute, volume, visualisation, metadata, media controls, scrobbling preserve behaviour | **Partial** — implemented and unit-tested; only audible playback and bitrate confirmed on real hardware. See `TESTING_NEEDED.md` |
| Network consolidation inventory complete, framework-owned exceptions explicit | Met — exceptions are webview `<img>` artwork and the updater plugin's own download |
| MPV and capture-specific dependencies/configuration removed from production | Met |
| Windows updater and all supported desktop bundles validated | **Not met** — no Windows or macOS testing has occurred |
| Platform matrix has final manual hardware evidence for Windows/macOS | **Not met** — both remain PENDING MANUAL VALIDATION |
| Progress report names exact tests, limitations and unavailable platform evidence | Met |

### Dependency duplication, as the plan requires documenting

`Cargo.lock` contains **two Reqwest versions**: 0.12.28 (the app, pinned) and 0.13.4, pulled by
`tauri-plugin-updater 2.10.1`. This predates the migration and is not introduced by it.

Removing it would mean either upgrading the app to Reqwest 0.13 purely to match a transitive
dependency — which the plan explicitly warns against doing blindly — or replacing the updater
plugin, which the plan equally warns against. The cost is a second HTTP client compiled into
the binary and used only for update downloads. Left as-is, deliberately.

## Validation rules

Record actual commands and outcomes here as work completes. A compile or synthetic PCM test is not physical audio validation. See `symphonia-platform-acceptance.md` for manual gates. Never mark M5 complete with outstanding required hardware checks.
