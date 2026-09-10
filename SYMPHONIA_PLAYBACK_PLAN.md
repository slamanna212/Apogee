# Apogee: Symphonia playback migration plan

Prepared 2026-09-10. This is an implementation handoff, not a report of completed playback testing.

## 1. Objective and decisions

Replace the external MPV process with an in-process Rust audio engine. Preserve Apogee's desktop behavior on Windows, macOS, and Linux while removing MPV installation, bundling, IPC, and process-lifetime management.

Use this stack as the implementation baseline:

| Responsibility | Choice |
| --- | --- |
| HTTP and HTTPS throughout application-controlled networking | Reqwest with Rustls, on Tokio |
| HLS protocol client | hls-runtime, using its caller-driven client core |
| MPEG-TS parsing and audio extraction | mpeg2ts-reader |
| MP3/AAC decoding | Symphonia |
| Sample-rate conversion | Rubato |
| Audio devices and output | CPAL |
| EQ, volume, mute, buffering, session lifecycle | Apogee Rust code |
| Spectrum visualization | Existing rustfft processing adapted to playback samples |

The user prefers hls-runtime over hls_client. Do not substitute hls_client merely because it has a simpler example. Validate hls-runtime's released API and actual capabilities first; broader documentation is not proof that every HLS feature works. If a required feature is absent, isolate and document the gap and complete independent work before seeking a product decision.

Scope is MP3/AAC radio delivered through direct MPEG-TS or HLS. Broad video or exotic-codec support is not a selection criterion. Future mobile reuse is desirable, but iOS/Android releases, background modes, and mobile UI work are outside this migration.

Do not introduce an FFmpeg runtime, libmpv fallback, external player, or permanent dual-engine setting into the final product. MPV may remain temporarily for development comparisons until the replacement meets the release criteria.

## 2. Instructions for the implementing agent

1. Read current repository instructions and this entire document before editing. Source files may have changed since this plan was written.
2. Run `git status --short`; preserve unrelated work. At preparation time, release.yml and dev-prerelease.yml already had local modifications. Never replace those files wholesale.
3. Follow milestones in dependency order. Keep the application buildable at each milestone. Do not delete MPV before the replacement has demonstrated functional parity.
4. Treat module names and contracts below as proposed design, not existing APIs. Verify every third-party method, feature flag, Send/Sync bound, and minimum Rust version against the selected release. Do not invent APIs from prose documentation.
5. Keep a running record in `docs/symphonia-migration-progress.md`: versions selected, completed milestones, commands and outcomes, remaining gaps, and platform checks not yet run.
6. Add tests for real behavior, especially cancellation and protocol boundaries. Do not replace existing regression tests with mocks that merely return success.
7. Implement authorized local work autonomously. Do not publish, deploy, or contact services' operators as part of this plan. Do not treat a missing platform runner as a reason to stop all independent implementation.
8. Never claim live/provider or cross-platform validation that was not performed. A headless test proves pipeline behavior, not that speakers work.

## 3. Repository findings to preserve

Read these files before implementing the related milestone:

| Existing location | Relevant behavior |
| --- | --- |
| `src-tauri/src/mpv.rs` | Process lifecycle, redacted logging, output devices, EQ, event forwarding |
| `src/lib/mpvClient.ts` | Frontend MPV command/event wrapper |
| `src/stores/playerStore.ts` and its tests | Playback state, retry/fallback, mute/volume, bitrate, OS media controls |
| `src/types/player.ts` | Current frontend status and displayed player fields |
| `src/lib/xtream.ts` and its tests | Provider API and URL construction |
| `src/lib/stellarTunerLog.ts` and its tests | Metadata endpoints and authentication differences |
| `src/lib/fetchWithTimeout.ts` | Current frontend HTTP abstraction |
| `src-tauri/src/lastfm.rs`, `notifications.rs` | Existing direct Reqwest clients |
| `src/stores/settingsStore.ts`, `src/pages/Settings.tsx` | Saved MPV device identifiers, EQ, diagnostics |
| `src/lib/equalizer.ts`, `src-tauri/src/mpv.rs` | EQ bands, presets, normalization, existing filter semantics |
| `src-tauri/src/waveform.rs`, `waveform/*.rs`, `src/lib/waveform.ts`, `src/components/Waveform.tsx` | FFT and system-audio capture |
| `src-tauri/src/lib.rs`, `updater.rs` | Startup, shutdown, Windows job-object/update interactions |
| `src/lib/mediaSession.ts`, `src/lib/discordRpc.ts`, `src/stores/scrobblingStore.ts` | Playback consumers whose semantics must remain correct |
| `scripts/fetch-mpv.mjs`, `src-tauri/tauri*.conf.json`, `.github/workflows/*.yml` | Build and packaging removal points |
| `docs/milestone-0-findings.md` | Previously observed provider behavior |

Important facts:

- The previously tested provider returned raw MPEG-TS for BOTH `.ts` and `.m3u8` URLs. File extensions are hints, not a reliable format discriminator. Do not copy its private endpoint/account values into fixtures or new documentation.
- Some initial requests time out while the upstream station starts. Preserve bounded retry behavior.
- Current fresh connection attempts alternate `.ts`, `.m3u8`, `.ts`, `.m3u8`, with four attempts, a 1.5-second retry delay, and a 20-second connect-to-play timeout. Use these as initial migration defaults unless testing justifies a documented change.
- Stop/pause for live radio releases playback. Resume reconnects to live; there is no DVR pause buffer.
- A successful play command does not mean playback has begun. The UI currently waits for actual playback confirmation.
- Metadata comes separately from StellarTunerLog. Its now-playing and channels calls are keyless; history uses an API key. Preserve this distinction.
- Volume writes to settings are debounced. EQ uses 10 bands at 31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, and 16000 Hz, with gains clamped to -12..12 dB.
- Saved audio devices currently use MPV identifiers. These are not CPAL identifiers.
- The Windows updater has custom installer handling because MPV management placed the app in a kill-on-close job object. Removing that object requires reviewing the installer path too.

## 4. Architecture and module boundaries

```text
React / Zustand
    | typed commands and playback snapshots/events
Tauri command adapter
    | requests
Playback controller ---------------------------------------+
    |                                                     |
    +-- Direct HTTP source --+                            | settings
    +-- HLS source ----------+                            | cancellation
         hls-runtime         |                            | recovery
         Reqwest fetches     v                            |
                       content routing                    |
                             |                            |
                      TS / ADTS / MP4 adapter              |
                             | compressed audio + timing   |
                       Symphonia worker                    |
                             | PCM                         |
                    channels + resampling                  |
                             |                            |
                      bounded PCM ring                    |
                             |                            |
                 CPAL callback: EQ / volume <--------------+
                             |
                      OS output device
                             |
                   nonblocking sample tap
                             v
                    FFT analysis worker --> UI spectrum
```

Suggested new files; keep closely related items together if separate files add no value:

```text
src-tauri/src/network.rs                 shared HTTP policy and clients
src-tauri/src/xtream.rs                  typed provider API commands
src-tauri/src/stellar.rs                 typed metadata API commands
src-tauri/src/playback/mod.rs            public engine facade and Tauri adapters
src-tauri/src/playback/types.rs          commands, snapshots, errors, device descriptors
src-tauri/src/playback/controller.rs     session ownership, retries, state transitions
src-tauri/src/playback/source/mod.rs     source events and bounded content detection
src-tauri/src/playback/source/http.rs    direct streaming source
src-tauri/src/playback/source/hls.rs     hls-runtime driver with Reqwest transport
src-tauri/src/playback/demux.rs          MPEG-TS/audio framing bridge
src-tauri/src/playback/decode.rs         Symphonia worker
src-tauri/src/playback/output.rs         CPAL owner, PCM ring, channel conversion/resampling
src-tauri/src/playback/dsp.rs            EQ, gain ramps, output protection
src-tauri/src/playback/analysis.rs       PCM-to-spectrum worker
src/lib/playerClient.ts                 typed invoke/listen facade
```

Keep core processing independent of Tauri AppHandle. Emit domain events through an injected sink/channel and translate them at the boundary. Do not design a general multimedia framework or a plugin system. Narrow interfaces for source, decoder, output, and events are sufficient.

### Ownership and scheduling

- One controller owns the current session and desired playback settings.
- Tokio tasks perform network requests and drive HLS timers/actions.
- A dedicated worker owns synchronous demux/decoder/resampler state. Do not run blocking decoding on Tokio executor workers. Abort of a spawn_blocking handle does not stop a running worker: give the worker cooperative cancellation and wakeable input.
- A CPAL owner keeps output stream lifetime and device configuration on a suitable thread. Inspect actual platform thread-safety constraints; do not fix trait errors by adding unsafe Send/Sync implementations.
- Preallocated bounded queues join stages. Backpressure and cancellation must work while a queue is full or empty.
- The audio callback never waits on network/decoder work, takes a blocking mutex, allocates, logs, emits Tauri events, or performs FFT analysis. Use a vetted bounded SPSC ring or equivalent; select and record its crate/version during milestone 0.
- Control updates use atomics or bounded nonblocking messages. Precompute EQ coefficients away from the callback and apply/smooth them without allocation.
- Spectrum analysis has its own bounded queue. Dropping analysis data is acceptable; blocking playback for analysis is not.

## 5. Dependency validation before implementation

Create `docs/symphonia-dependency-validation.md` with exact versions, features, MSRV, license identifiers, relevant APIs, transitive native dependencies, and results of a small compile proof.

Validate:

1. **hls-runtime:** locate the actual published repository/source and client examples. Verify ordinary live HLS as well as its advertised low-latency model. Determine how it handles master playlists, audio renditions, media sequence, redirects/relative URLs, initialization segments, byte ranges, discontinuities, gaps, end lists, encryption, and cancellation. Record supported, unsupported, and untested separately. Verify the media output type: bytes, resources, or already parsed frames; adapt to that actual API rather than assuming bytes.
2. Use hls-runtime's caller-driven core so requests pass through network.rs. If its Tokio adapter supports injecting the required client/policies, it may be used after verification. Avoid a hidden second HTTP stack and unnecessary server/origin dependencies where features permit.
3. **Symphonia:** enable only needed MP3/AAC and container features. Confirm profile and framing on a representative provider sample. Its published matrix distinguishes AAC-LC from HE-AAC variants. This is one concrete compatibility check, not a reason to broaden codec scope. Document a real unsupported case instead of silently degrading it.
4. **mpeg2ts-reader:** compile a PAT/PMT plus elementary-stream callback example with the chosen release. Establish how continuity errors and PES timestamps are exposed.
5. **CPAL:** promote the existing Windows-only dependency to applicable desktop targets. Validate output sample formats, device identifiers, backend features, and minimum supported OS versions.
6. **Rubato:** confirm block sizes, reusable buffers, latency, and the conversion API for the chosen release. Do not assume APIs from older versions.
7. **Reqwest:** the repository currently pins 0.12.28 with Rustls. Prefer a compatible version shared with retained Tauri components; do not upgrade to latest blindly. Inspect `cargo tree -d` and the relevant feature tree. If a duplicate version is unavoidable, document why and its cost.
8. Rust MSRV currently declares 1.77.2. Verify the actual toolchain/CI setup and reconcile it with chosen dependencies. Update a toolchain declaration and CI together if required; do not leave a false rust-version claim.

No live credentials in source, fixtures, logs, or reports. Use existing authorized local configuration if available. Otherwise implement with local fixtures and clearly leave provider validation outstanding. Fixture generation can use development tools such as FFmpeg; that does not make them runtime dependencies.

## 6. Shared networking design

Introduce a managed NetworkService with reused Reqwest clients and explicit policies. Share TLS/proxy/user-agent construction. Separate client profiles where their settings differ; one library does not mean one global timeout or one credential pool.

| Traffic | Required policy |
| --- | --- |
| JSON API | Finite overall deadline, body-size bounds, clear status errors |
| Artwork | Finite deadline, byte limit, content validation, cache reuse |
| Continuous TS | Connect deadline, stalled-read detection, no total lifetime timeout |
| HLS playlists | Finite deadline, bounded body, refresh/cache behavior appropriate for live data |
| HLS segments | Bounded concurrency and memory, deadline, byte-range handling when needed |

- Make network waits and retry sleeps cancellable.
- Credential-bearing URLs and headers must be redacted before logging. Reqwest error strings may contain URLs; sanitize errors too.
- Restrict media requests to HTTP/HTTPS, including redirected/nested resources. Preserve support for local/private provider addresses; do not impose a public-host-only rule.
- Resolve relative HLS resource references against the effective response URL after redirects. Forward provider-specific credentials/headers according to explicit origin rules; do not indiscriminately forward secrets to every host.
- Do not globally disable certificate verification or change existing HTTP URL behavior under this migration.
- Bound retry work across layers: hls-runtime resource retries and controller reconnects must not create an unbounded nested retry loop.
- Retain update signature verification and plugin-managed installation. A single application HTTP implementation does not justify reimplementing the updater.

Migrate Xtream and Stellar API operations behind typed Rust commands while preserving existing TS-facing return shapes. Move Last.fm and artwork clients to NetworkService. Maintain tests for URL encoding, auth differences, response errors, and timeouts.

Inventory remaining network entry points, including browser `<img>` loads and updater/plugin traffic. For remote artwork under application control, prefer a Rust cache that returns local asset URLs, with bounded storage and deduplicated downloads. Verify Tauri asset scopes and retain placeholders. Explicitly document unavoidable framework-owned traffic; do not claim all networking is shared if it is not. Remove plugin-http only after its callers/capabilities have been migrated.

## 7. Source detection and HLS

### Detection rules

Use content type plus a bounded prefix probe. Retain every probed byte and replay it into the selected pipeline.

- `#EXTM3U` identifies a playlist after supported leading BOM/whitespace handling.
- Repeated valid TS packet framing identifies MPEG-TS. Account for arbitrary HTTP chunk boundaries; a single 0x47 byte is insufficient evidence.
- Recognized ADTS/MP3/MP4 framing routes to the appropriate supported reader.
- HTML/JSON error responses with HTTP 200 must become actionable source errors, not endless decoder probing.
- Bound probe bytes and elapsed time. Distinguish incomplete input from definitive unsupported input.
- Specifically test `.m3u8` returning chunked raw TS and a misleading MIME type.

### HLS adapter

Drive hls-runtime with cancellable network operations and monotonic time. Bound playlist size, outstanding resource count, resource bytes, and retained live history.

Select an audio rendition deterministically when alternatives exist. Use the library's live scheduling and sequencing rather than writing a competing scheduler around it. Ensure completed requests cannot reorder playback.

Normal segment boundaries should not reset a continuous decoder. Actual discontinuities, codec changes, or invalid continuity require a deliberate flush/reset and an explicit rebuffer event. For fragmented MP4, retain initialization data and use an integration proven to handle fragments; do not assume ordinary MP4 support automatically proves live fMP4 support. Validate decryption capabilities if an encountered playlist needs them; recognizing an encryption tag is not decryption support.

Use local fixture playlists for ordinary HLS even if the available provider happens to return only raw TS. Do not mark HLS implemented based on successful `.m3u8` URLs that actually contain TS.

## 8. MPEG-TS and Symphonia bridge

The demux adapter must discover programs/tracks, select the intended audio track, and reconstruct elementary audio. Avoid retaining video payload if present.

- Handle PAT/PMT repetition and updates.
- Retain partial TS, PES, and audio frames across incoming chunks.
- PES packet boundaries are not audio-frame boundaries.
- Preserve relevant timing and discontinuity metadata alongside compressed frames.
- On damaged continuity, discard affected partial frames and resynchronize; never concatenate known-corrupt fragments into valid-looking audio.
- Support the observed AAC transport framing explicitly (e.g. ADTS). If a sample uses LATM/LOAS, record and resolve that adapter requirement; do not feed it into an ADTS reader.
- Choose either a small Symphonia FormatReader bridge or its existing elementary-stream reader behind a cancellable input adapter. Prove the chosen API with a fixture before building abstractions around it.
- Distinguish temporary lack of network bytes from end-of-stream. A reader returning zero bytes commonly signals EOF and must not be used for a temporary empty queue.
- A decoder format/configuration change must reconfigure downstream conversion safely and flush incompatible queued samples.

Track compressed audio bytes and decoded duration for displayed bitrate. Do not label total TS/HTTP throughput as audio bitrate. Allow unknown bitrate until enough evidence exists.

## 9. Playback contracts and state

Expose typed commands rather than arbitrary MPV property strings. Proposed facade:

```text
player_play(request) -> acknowledged session identity
player_stop(request identity) -> acknowledgement
player_set_volume(0..100)
player_set_muted(bool)
player_set_equalizer(enabled, gains[10])
player_list_devices() -> device descriptors
player_set_device(optional device identity)
player_get_snapshot() -> current state and settings
```

For play, accept a typed station/source request sufficient for Rust to build both provider URL candidates. Keep credentials only in memory/keyring-backed configuration, never in snapshots. Migrate URL construction with existing escaping tests; do not lose provider base-path handling.

Use one serialized controller command stream. Assign a monotonically increasing generation to every accepted play/stop so old asynchronous completions cannot win. Include a frontend request token in acknowledgements/events to handle rapid selections and events arriving before invoke resolves. Do not rely on channel ID alone: selecting the same station twice creates different sessions.

Snapshots/events should include generation, monotonic event revision, station identifier, state, buffering reason, attempt count, audio format/bitrate when known, actual output device, and structured sanitized error. Register listeners before submitting commands; use revisioned snapshots for initialization/resynchronization so an older snapshot cannot overwrite a newer event.

Internal states: Stopped, Connecting, Buffering, Playing, Recovering, Failed. Map these to the existing frontend status/isBuffering shape where possible. The frontend is a projection of Rust state, not a second retry/state machine.

Playback is confirmed when CPAL starts consuming valid queued samples for the session, not when HTTP succeeds or decoding starts. Digital silence is valid playback; do not require a nonzero amplitude. Output callbacks can set a flag/counter; a non-real-time task publishes confirmation.

Stop must invalidate in-flight work, cancel requests/timers, wake blocked workers, clear queues, and stop output. Resume starts a fresh live connection. Rapid station changes must not briefly emit old queued audio or old metadata state.

Initial retry defaults preserve the current four-attempt alternating-extension behavior. Preserve a known-good URL preference for resume/recovery, then use bounded fallback as appropriate. Classify permanent authentication/unsupported-content errors so they do not loop forever. Reset recovery budgets only after a documented stable-play interval, not every successful HTTP response.

## 10. Output, DSP, devices, visualization

### Buffering and resampling

Use a bounded PCM ring sized in frames for the selected output format. Start with configurable internal thresholds (for example, 500 ms initial PCM target and a 2-second PCM capacity) and tune against tests; these are proposals, not latency guarantees. HLS segment buffering adds separate latency and needs a separate bound. Start/rebuffer thresholds should avoid rapid state oscillation.

Handle mono/stereo mapping explicitly and query device-supported sample formats/rates. Prefer the device's appropriate default configuration; do not force 48 kHz or float output everywhere. Convert final samples safely to the selected device type. Reuse resampler buffers. Do not implement clock-drift correction until measurement establishes a need; keep bounded latency and recovery observable.

On underrun, emit silence without blocking and notify the controller outside the callback. Distinguish output failure from network starvation. Stop unnecessary decoding/analysis while stopped. Define suspend/resume and long-stall recovery so the app returns near live rather than draining stale data for minutes.

### EQ and gain

Preserve 10-band presets and stored settings. Read current MPV filter width/Q behavior before choosing equivalent biquad coefficients. Use tested filter math or a suitable reviewed Rust DSP crate. Include per-channel filter state, coefficient/gain smoothing, denormal handling as appropriate, and safe behavior for frequencies at/above Nyquist on low-rate devices.

Apply EQ and user gain near consumption so volume/mute changes are not delayed by seconds of queued processed audio. Precompute coefficients outside the callback. Add explicit headroom/output protection, document its behavior, and test boosts for clipping; do not claim bit-exact MPV parity. Bypassed/flat processing should not unexpectedly change level. Mute preserves the stored volume.

### Device selection and persistence

Provide device descriptors with identity, display name, backend, and default status where supported. Verify persistence guarantees for the selected CPAL release. Never assume a display name is globally unique.

Version the audio-device setting. Migrate old MPV selection by a verified mapping or unique descriptive match; otherwise use system default and show/log a concise explanation. Preserve the rest of settings. Track requested and effective device separately so a temporary unplug does not unnecessarily erase the user's preference.

Switch devices without leaving a stale callback alive; flush/reconfigure conversion if output format changes. If the selected device disappears, attempt system default. If no output exists, expose a recoverable state and allow retry without restarting the app. Implement or verify default-device-change observation/polling when following system default; enumeration alone does not follow changes.

### Visualizer

Tap post-EQ/post-volume samples submitted to output. Analyze on a worker and preserve the existing UI spectrum event shape where practical. Match analysis sample rate/channel handling to real output. Decimate or drop analysis data when necessary. The visualizer reflects Apogee output before later OS/hardware effects.

Retain useful FFT normalization/smoothing from waveform.rs but re-evaluate capture-specific compensation. Remove loopback/process-tap capture once parity is verified. No microphone/system-audio capture permission should be needed for the new spectrum path.

## 11. Ordered milestones and exit criteria

### M0 — Inventory, dependencies, and fixtures

- [ ] Read mapped files, preserve existing changes, record baseline frontend/Rust checks.
- [ ] Complete dependency validation and build a minimal hls-runtime compile proof.
- [ ] Record observed codec/profile/framing without secrets.
- [ ] Create local TS, real HLS, and mislabeled-URL fixtures plus a deterministic HTTP fixture server.
- [ ] Establish clean build/toolchain requirements for each desktop target.

Exit: dependencies resolve; actual APIs and material gaps are documented; fixtures do not require a subscription or internet connection.

### M1 — Networking service

- [ ] Add NetworkService with profiles, cancellation, body bounds, and redaction.
- [ ] Migrate direct Rust clients and add typed provider/metadata commands.
- [ ] Preserve frontend API shapes and authentication behavior.
- [ ] Begin artwork/framework traffic inventory; finish artwork migration before final cleanup.

Exit: API tests pass; one configurable application HTTP service exists; no live audio lifetime timeout is inherited from API policy.

### M2 — Headless source-to-PCM pipeline

- [ ] Implement content detection and direct TS ingestion.
- [ ] Implement TS-to-Symphonia bridge and decode into a test sink.
- [ ] Implement hls-runtime transport adapter and genuine HLS fixture playback.
- [ ] Preserve timing/reset information and bounded queues.

Exit: MP3/AAC fixtures yield correctly timed PCM; successive HLS segments do not introduce artificial decoder resets/gaps; mislabeled `.m3u8` TS works.

### M3 — CPAL and controller

- [ ] Add CPAL output owner, conversion/resampling, bounded ring, and underrun reporting.
- [ ] Implement generation-safe controller, snapshots/events, cancellation, stop/resume, retry/fallback.
- [ ] Add device enumeration, default fallback, and hotplug recovery.
- [ ] Prove audio on the available desktop and schedule real-device checks for the others.

Exit: station switch/stop cancels every stage; actual output confirms Playing; invalidated sessions cannot emit audio/state; retries are bounded.

### M4 — Feature parity

- [ ] Add EQ, smooth volume/mute, saved-device migration, and direct PCM visualizer.
- [ ] Preserve bitrate semantics and structured diagnostics.
- [ ] Replace mpvClient with playerClient and simplify playerStore.
- [ ] Remove frontend MPV polling and duplicate retries.
- [ ] Verify OS controls, sleep timer, media metadata, Discord presence, Last.fm/scrobbling, and notification behavior.

Exit: existing user-visible functions work; scrobbling counts real playback rather than connecting/recovering; stopped state has no background capture work.

### M5 — Cleanup and packaged validation

- [ ] Remove MPV runtime code, frontend wrappers, build downloads/resources, package dependencies, and obsolete capture code.
- [ ] Review and update shutdown/update installer paths as a single coherent change.
- [ ] Finish networking migration and remove unused plugin-http/dependencies/capabilities.
- [ ] Update docs/settings descriptions and CI prerequisites.
- [ ] Complete platform acceptance matrix and long-session checks.

Exit: distributable desktop builds run without MPV/FFmpeg installed; no active old-engine/capture paths remain; unrun platform checks are explicitly outstanding, not silently passed.

## 12. Cleanup details that are easy to miss

- `src-tauri/src/lib.rs`: remove MPV state/handlers/job object and capture startup; initialize NetworkService and player; ensure bounded shutdown.
- `src-tauri/src/updater.rs`: remove MPV kill calls and reassess CREATE_BREAKAWAY_FROM_JOB after removing the job object. Preserve installer launch-failure reporting, update verification, and install arguments. Verify installer survival and relaunch on Windows. Do not blindly replace the custom updater with plugin defaults.
- `src-tauri/tauri.conf.json`: remove MPV fetching from beforeBuildCommand and Linux MPV package dependencies. Add only genuinely required CPAL backend runtime dependencies verified by builds.
- `src-tauri/tauri.windows.conf.json`, `tauri.linux.conf.json`: remove bundled MPV resources while preserving other configuration.
- `scripts/fetch-mpv.mjs`, `package.json`, lockfiles: remove obsolete fetch script and 7zip-min/undici only if no other caller needs them.
- `src-tauri/Cargo.toml`: remove win32job and capture-only libraries/features only after finding every use. Preserve windowing/notification dependencies. “Rust audio stack” does not mean no platform libraries or native crypto primitives.
- macOS entitlements/configuration: remove obsolete MPV/capture explanations and permissions where no longer used. Do not enable App Sandbox or alter unrelated signing policy as incidental cleanup.
- `Settings.tsx`, `Waveform.tsx`, README.md, CLAUDE.md: replace obsolete playback/capture/installation instructions. Keep historical findings clearly historical.
- CI: inspect all release/dev/PR workflows and build scripts for MPV downloads, caching, package installs, and resource assumptions. Preserve existing signing, publishing, and trusted-runner boundaries.

## 13. Verification plan

### Automated behavioral tests

Use generated or redistributable short fixtures and a local HTTP server. No paid/provider endpoints in CI.

| Area | Required cases |
| --- | --- |
| Detection | TS at `.m3u8`; playlist at unexpected extension; wrong MIME; chunk-split prefix; HTML error body; bounded probe |
| Direct HTTP | Chunked indefinite body; redirects; initial timeout then success; stalled read; cancellation while waiting |
| TS/framing | Arbitrary byte chunk boundaries; partial PES/audio frames; continuity gap; PMT update; non-audio packets |
| Decode | MP3 and supported AAC fixtures; correct duration/rate/channels; corrupt frame recovery; config change |
| HLS | Real sliding live playlist; master-to-media selection; relative URL after redirect; duplicate reload; ordered segment output; missing segment; discontinuity; end list; required init/byte-range cases |
| Controller | A-to-B race; A-to-A race; stop during connect/retry/full queue; stale snapshot/event; bounded retries; permanent error |
| Output/DSP | Queue underrun; conversion bounds; mono/stereo; flat/bypass response; known EQ response; smoothing; low-rate Nyquist handling; finite samples |
| Persistence | Existing settings without schema version; legacy MPV device; ambiguous/missing device; EQ and volume preserved |
| Integration | Media controls; no scrobble credit while connecting; stale session cannot update presence; sleep timer stop |
| Diagnostics | Credentials absent from URLs, headers, nested HTTP errors, and exported logs |

Compare decoded duration and waveform characteristics with a known reference where useful; allow numerical tolerance rather than requiring identical lossy-decoder floating-point output. Test sample timing at HLS boundaries so “sound came out” does not conceal gaps or duplicates.

### Commands

Run from repository root unless noted:

```bash
npm test
npm run lint
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
```

Use existing platform build scripts/config overlays for native bundles; inspect them before choosing exact commands. Record baseline warnings separately from introduced failures. If adding an isolated core crate, explicitly test it too. Run tests appropriate to each milestone rather than repeating full packaging after every small edit.

### Real desktop acceptance

Run on Windows x64, macOS Apple Silicon and Intel if both remain supported, and the supported Linux packaging environments:

- Clean install without MPV/FFmpeg or Homebrew media packages.
- Direct TS and genuine HLS audible output.
- Stop/switch response target: no stale application audio after roughly 250 ms under normal conditions, acknowledging OS/device buffer latency; measure and record exceptions.
- Volume/mute/EQ changes respond promptly without clicks or clipping regressions.
- Unplug/replug headphones or USB output; switch default output; handle no available device.
- Suspend/resume and a 30-second network outage recover or fail clearly within the configured budget.
- Run a two-hour session on each primary desktop platform; record process memory, queue occupancy, underruns, reconnects, and any monotonic memory growth. Bound queues by construction; do not invent a universal RSS budget before measuring.
- Validate visualization against the station, including digital silence and mute.
- Quit while connecting/playing; no leaked network task or hanging shutdown.
- Validate update download/install/relaunch, especially Windows installer survival.
- Test existing saved settings and keyring credentials through upgrade.

## 14. Completion checklist

- [ ] Both source paths use the same application HTTP service and converge on the Rust audio pipeline.
- [ ] hls-runtime is actually integrated and tested with real playlist fixtures.
- [ ] No extension-only routing assumption remains.
- [ ] MP3/AAC scope is verified against documented profile/framing fixtures.
- [ ] One controller owns retries, cancellation, and playback truth.
- [ ] Bounded buffering and session generations are tested under adversarial timing.
- [ ] Output devices, EQ, mute, volume, visualization, metadata, media controls, and scrobbling preserve behavior.
- [ ] Network consolidation inventory is complete, with framework-owned exceptions explicit.
- [ ] MPV and capture-specific dependencies/configuration are removed from production.
- [ ] Windows updater and all supported desktop bundles are validated.
- [ ] Progress report names exact tests, remaining limitations, and any unavailable platform evidence.

## 15. Primary references

These links informed the design. Verify selected release documentation during implementation rather than relying on a changing `latest` page.

- [Symphonia repository and support matrix](https://github.com/pdeljanov/Symphonia)
- [Symphonia format readers](https://docs.rs/symphonia/latest/symphonia/default/formats/index.html)
- [hls-runtime crate documentation](https://docs.rs/hls-runtime/latest/hls_runtime/)
- [hls-runtime client interface](https://docs.rs/hls-runtime/latest/hls_runtime/client/struct.HlsClient.html)
- [mpeg2ts-reader](https://docs.rs/mpeg2ts-reader/latest/mpeg2ts_reader/)
- [Elementary stream callbacks](https://docs.rs/mpeg2ts-reader/latest/mpeg2ts_reader/pes/trait.ElementaryStreamConsumer.html)
- [Reqwest](https://docs.rs/reqwest/latest/reqwest/)
- [Tauri HTTP plugin and Reqwest relationship](https://docs.rs/tauri-plugin-http/latest/tauri_plugin_http/)
- [Rubato](https://docs.rs/rubato/latest/rubato/)
- [CPAL](https://docs.rs/cpal/latest/cpal/)
- [stream-download, optional rather than baseline](https://docs.rs/stream-download/latest/stream_download/)

Some hls-runtime subpages were intermittently unavailable during research. Its top-level documentation establishes the advertised architecture, not a completed source audit. Milestone 0 must inspect the actual published implementation before coding the adapter.
