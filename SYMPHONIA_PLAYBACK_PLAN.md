# Apogee: Symphonia playback migration plan

Prepared 2026-09-10. This is an implementation handoff, not a report of completed playback testing.

## 1. Objective and decisions

Replace the external MPV process with an in-process Rust audio engine. Preserve Apogee's desktop behavior on Windows, macOS, and Linux while removing MPV installation, bundling, IPC, and process-lifetime management.

Use this stack as the implementation baseline:

| Responsibility | Choice |
| --- | --- |
| HTTP and HTTPS throughout application-controlled networking | Reqwest with Rustls, on Tokio |
| HLS protocol client | hls-runtime, using its caller-driven client core |
| Direct MPEG-TS parsing and audio extraction | transmux::StreamingTsDemux, sharing the implementation used by hls-runtime |
| Container detection | container-probe from the same workspace, after released-API validation |
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
React / Zustand <--> Tauri adapter <--> Playback controller
                                            | commands/settings/cancellation
Shared Reqwest service                      |
    |                                       |
Bounded content detection (container-probe)  |
    |                                       |
    +-- Direct TS --> StreamingTsDemux ------+--> compressed-sample adapter
    |                                       |        |
    +-- HLS --> hls-runtime -----------------+        v
                internal TS/fMP4 demux          Symphonia decoder
                Output::Samples                      |
                                               channels + Rubato
                                                     |
                                               bounded PCM ring
                                                     |
                                         CPAL callback: EQ / volume
                                                     |
                                                OS audio output
                                                     |
                                           nonblocking sample tap
                                                     v
                                          FFT worker --> UI spectrum
```

**Demux exactly once.** hls-runtime's reviewed client contract emits `Output::Samples`: demuxed compressed access units, not PCM and not container bytes. Its internal transmux TS/fMP4 demuxers already perform container extraction. Route those samples directly into the common compressed-sample adapter. Never pass them through a second TS/ADTS/MP4 demuxer or repackage them merely to feed a file reader.

For direct endless TS connections, use transmux's incremental `StreamingTsDemux`; `TsDemux` is the batch wrapper, not the default interface for an endless source. Pin compatible versions shared with hls-runtime so both paths use the same demux implementation. Drop mpeg2ts-reader from the baseline. If a selected release differs from the reviewed contract, document its actual output boundary before proceeding; the invariant remains one demux stage per path.

The convergence point is compressed access units plus codec configuration, track identity, timing, and discontinuity events. Using shared code reduces divergence but does not prove identical state handling: test continuous input and segmented input against each other.

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
src-tauri/src/playback/demux.rs          direct StreamingTsDemux adapter
src-tauri/src/playback/samples.rs        shared access-unit/config/timing adapter
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

1. **hls-runtime:** locate the actual published repository/source and client examples. Verify ordinary live HLS as well as its advertised low-latency model. Determine how it handles master playlists, audio renditions, media sequence, redirects/relative URLs, initialization segments, byte ranges, discontinuities, gaps, end lists, encryption, and cancellation. Record supported, unsupported, and untested separately. Confirm the selected release's `Output::Samples` payload and internal transmux dependency. Record where codec configuration, timescales, track changes, and discontinuities are exposed. Write a compile proof that translates its output directly to the common compressed-sample contract without any container demuxer downstream.
2. Use hls-runtime's caller-driven core so requests pass through network.rs. If its Tokio adapter supports injecting the required client/policies, it may be used after verification. Avoid a hidden second HTTP stack and unnecessary server/origin dependencies where features permit.
3. **Symphonia:** enable only needed MP3/AAC and container features. Confirm profile and framing on a representative provider sample. Its published matrix distinguishes AAC-LC from HE-AAC variants. This is one concrete compatibility check, not a reason to broaden codec scope. Document a real unsupported case instead of silently degrading it.
4. **transmux and container-probe:** select releases compatible with hls-runtime. Compile an incremental StreamingTsDemux example; verify MP3 and required AAC framing, track configuration, continuity events, timestamps, and bounded state on endless input. Confirm container-probe's incremental API, ambiguity/need-more-data results, and support for 188/192/204/208-byte TS layouts. These are source-review requirements, not assumed passing tests. Prefer these existing implementations to new packet detection or parsing code. Check dependency resolution to avoid duplicate incompatible transmux types/versions.
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

Use content type plus a bounded prefix probe implemented with container-probe. Retain every probed byte and replay it into the selected source/demux path. Use the library's confidence/ambiguity results; request more data within the bound rather than guessing. Verify the released API in M0.

- `#EXTM3U` identifies a playlist after supported leading BOM/whitespace handling.
- Use container-probe's TS stride/phase detection for supported 188/192/204/208-byte packet layouts. Test arbitrary HTTP chunk boundaries, ambiguous prefixes, and false sync bytes. Normalize framing only if required by the selected StreamingTsDemux API; do not write a competing detector.
- Raw ADTS/MP3/MP4 content, if encountered directly, needs its own verified adapter before the common access-unit boundary. This does not add a downstream demuxer to HLS sample output. Do not broaden scope speculatively.
- HTML/JSON error responses with HTTP 200 must become actionable source errors, not endless decoder probing.
- Bound probe bytes and elapsed time. Distinguish incomplete input from definitive unsupported input.
- Specifically test `.m3u8` returning chunked raw TS and a misleading MIME type.

### HLS adapter

Drive hls-runtime with cancellable network operations and monotonic time. Bound playlist size, outstanding resource count, resource bytes, and retained live history.

Select an audio rendition deterministically when alternatives exist. Use the library's live scheduling and sequencing rather than writing a competing scheduler around it. Ensure completed requests cannot reorder playback.

Normal segment boundaries should not reset a continuous decoder. Forward the library's sample/configuration/discontinuity events into the common adapter. Actual discontinuities, codec changes, or invalid continuity require a deliberate flush/reset and an explicit rebuffer event. For fragmented MP4, hls-runtime's internal Fmp4Demux owns container/init-segment parsing; verify that its output supplies the decoder configuration and timing required by Symphonia. Do not add a Symphonia MP4 reader behind these samples. Validate decryption capabilities if an encountered playlist needs them; recognizing an encryption tag is not decryption support.

Use local fixture playlists for ordinary HLS even if the available provider happens to return only raw TS. Do not mark HLS implemented based on successful `.m3u8` URLs that actually contain TS.

## 8. Shared compressed-sample bridge to Symphonia

Direct TS: feed arbitrary network chunks into StreamingTsDemux and translate its events. HLS: translate hls-runtime's already-demuxed sample output. Both adapters must produce the same internal representation, with no second container parsing pass.

Define explicit records for track configuration, compressed access units, discontinuity/reset, and end-of-stream. Include codec/profile information, decoder initialization bytes, track identity, presentation/decode timestamps and timescale, and duration where available. Keep the representation narrow; do not replicate the entire transmux IR.

- Let the shared transmux implementation handle PAT/PMT, TS/PES assembly, and supported audio framing. Verify partial TS/PES/audio data survives input chunks and that retained state is bounded.
- Select the audio track deterministically and avoid retaining video payload unnecessarily.
- Verify support for observed MP3/AAC framing; if required LATM/LOAS is unsupported, document that specific gap rather than passing it to an ADTS reader.
- Map demuxed access units to Symphonia decoder packets with correct codec configuration. Verify whether AAC headers have already been stripped; neither strip twice nor prepend an ADTS/container wrapper by assumption.
- Translate timestamps/durations using explicit timebases. Handle unknown timing, wraparound, gaps, and track changes consistently on both paths.
- Forward meaningful continuity/reset events. Reset decoder/resampler and flush incompatible PCM when needed; never reset solely because another normal HLS segment arrived.
- Keep one decoder per selected track/session. Do not recreate it for each access unit or segment.
- Distinguish pending input from end-of-stream using channel/events. A temporarily empty queue is not EOF.
- Prove identical decoded duration and equivalent sample continuity when the same audio is fed as continuous TS and as HLS segments. This test is required even when both paths share a demux implementation.

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
- [ ] Create `docs/symphonia-platform-acceptance.md` in M0 with the matrix below, a named tester/device or explicit UNASSIGNED status for every platform, artifact delivery method, and planned manual checkpoints. Missing hardware access must be visible now, not discovered in M5.

Exit: dependencies resolve; actual APIs and material gaps are documented; fixtures do not require a subscription or internet connection.

### M1 — Networking service

- [ ] Add NetworkService with profiles, cancellation, body bounds, and redaction.
- [ ] Migrate direct Rust clients and add typed provider/metadata commands.
- [ ] Preserve frontend API shapes and authentication behavior.
- [ ] Begin artwork/framework traffic inventory; finish artwork migration before final cleanup.

Exit: API tests pass; one configurable application HTTP service exists; no live audio lifetime timeout is inherited from API policy.

### M2 — Headless source-to-PCM pipeline

- [ ] Implement content detection and direct TS ingestion.
- [ ] Implement StreamingTsDemux and the shared access-unit-to-Symphonia bridge; decode into a test sink.
- [ ] Implement hls-runtime transport adapter and genuine HLS fixture playback, feeding Output::Samples directly into the shared adapter.
- [ ] Compare continuous TS versus segmented HLS for the same audio; assert no duplicate demux, sample duplication, or boundary reset.
- [ ] Preserve timing/reset information and bounded queues.

Exit: MP3/AAC fixtures yield correctly timed PCM; successive HLS segments do not introduce artificial decoder resets/gaps; mislabeled `.m3u8` TS works.

### M3 — CPAL and controller

- [ ] Add CPAL output owner, conversion/resampling, bounded ring, and underrun reporting.
- [ ] Implement generation-safe controller, snapshots/events, cancellation, stop/resume, retry/fallback.
- [ ] Add device enumeration, default fallback, and hotplug recovery.
- [ ] Prove audio on the available physical desktop. Deliver test artifacts and instructions to the Windows/macOS manual testers identified in M0; run the first audible-output, stop/switch, and device enumeration checks now. Mark unavailable results PENDING MANUAL VALIDATION, never passed by CI.

Exit for core implementation: station switch/stop cancels every stage; actual output confirms Playing on tested hardware; invalidated sessions cannot emit audio/state; retries are bounded. Platform readiness is separate: Windows/macOS checks require manual evidence. Independent M4 work may continue while those results are pending; do not declare all-platform M3 acceptance.

### M4 — Feature parity

- [ ] Add EQ, smooth volume/mute, saved-device migration, and direct PCM visualizer.
- [ ] Preserve bitrate semantics and structured diagnostics.
- [ ] Replace mpvClient with playerClient and simplify playerStore.
- [ ] Remove frontend MPV polling and duplicate retries.
- [ ] Verify OS controls, sleep timer, media metadata, Discord presence, Last.fm/scrobbling, and notification behavior.
- [ ] Run manual Windows/macOS hotplug, default-device switching, and legacy-device migration checks against this milestone's artifact; record failures and fix before release readiness.

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
| TS/framing | StreamingTsDemux with arbitrary chunks; supported packet strides; partial PES/audio frames; continuity gap; PMT update; non-audio packets |
| Shared sample bridge | HLS samples bypass demux; configuration/timescale propagation; continuous TS versus segmented HLS equivalence; no resets at normal boundaries |
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

### Platform execution constraints and manual ownership

The current working-tree release/dev workflows have Linux builds on `rapture-apogee`, Windows builds on `windows-latest`, and macOS universal builds on `macos-latest`. Several orchestration jobs also use `rapture-apogee`. Reinspect the workflows in M0: the claim that all release builds use one runner does not match this checkout, and runner arrangements can change.

**Native compilation/packaging is not audio-device validation. Windows and macOS audible playback, hotplug, default-device following, and migration of real saved device selections are manual tests under this plan.** There is no verified automated physical-audio lab. A GitHub-hosted build, mock output device, or loopback-free headless fixture cannot pass those checks. Do not assume the self-hosted Linux runner has usable physical audio either.

Create and maintain this matrix from M0:

| Target | Automated evidence available/required | Manual evidence required | Initial status |
| --- | --- | --- | --- |
| Linux supported packages | Existing build runner; add/verify fixture and Rust checks | Audible output, devices/hotplug, suspend/resume, settings upgrade, long session on a physical desktop | UNASSIGNED until tester/device recorded |
| Windows x64 | Hosted native build; add/verify headless tests | WASAPI output, device switching/hotplug, old MPV device migration, media keys, installer/relaunch, long session | PENDING MANUAL VALIDATION |
| macOS Apple Silicon | Hosted universal build; add/verify headless tests | CoreAudio output, device switching/hotplug, old device migration, media keys, signed-app startup, long session | PENDING MANUAL VALIDATION |
| macOS Intel, if supported | Universal binary contains Intel slice; execution is not implied | Physical Intel execution/audio and upgrade smoke test; required device/long-session coverage | PENDING MANUAL VALIDATION |

For each result record tester, date, OS/architecture, physical audio device/backend, commit/artifact identity, test steps, observed behavior, and redacted diagnostics. Track build, headless tests, and manual tests in separate columns. Defaults are pending, not passing.

Checkpoints: M0 identifies hardware/test owners and unknowns; M3 requests the first audio/device smoke pass; M4 exercises DSP/hotplug/migration; M5 repeats release acceptance against final artifacts. If no tester/device is available, continue independent code and automated work, but retain an explicit release-readiness blocker for that platform. Do not mark the migration complete or remove support for the platform to make the checklist pass. Shipping with a missing acceptance result would require an explicit user decision, not an agent assumption.

### Real desktop acceptance

The following are manual unless a specific hardware automation setup and evidence have been documented. Run on Windows x64, macOS Apple Silicon and Intel if both remain supported, and the supported Linux packaging environments:

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

- [ ] Both source paths use the same application HTTP service and converge at compressed access units, with exactly one demux stage each.
- [ ] Direct TS and hls-runtime share compatible transmux code; mpeg2ts-reader is not a parallel baseline demuxer.
- [ ] hls-runtime is actually integrated and tested with real playlist fixtures.
- [ ] No extension-only routing assumption remains.
- [ ] MP3/AAC scope is verified against documented profile/framing fixtures.
- [ ] One controller owns retries, cancellation, and playback truth.
- [ ] Bounded buffering and session generations are tested under adversarial timing.
- [ ] Output devices, EQ, mute, volume, visualization, metadata, media controls, and scrobbling preserve behavior.
- [ ] Network consolidation inventory is complete, with framework-owned exceptions explicit.
- [ ] MPV and capture-specific dependencies/configuration are removed from production.
- [ ] Windows updater and all supported desktop bundles are validated.
- [ ] M0 platform matrix has final manual hardware evidence for Windows/macOS (including hotplug and legacy-device migration), clearly separated from CI build results.
- [ ] Progress report names exact tests, remaining limitations, and any unavailable platform evidence.

## 15. Primary references

These links informed the design. Verify selected release documentation during implementation rather than relying on a changing `latest` page.

- [Symphonia repository and support matrix](https://github.com/pdeljanov/Symphonia)
- [Symphonia format readers](https://docs.rs/symphonia/latest/symphonia/default/formats/index.html)
- [hls-runtime crate documentation](https://docs.rs/hls-runtime/latest/hls_runtime/)
- [hls-runtime client interface](https://docs.rs/hls-runtime/latest/hls_runtime/client/struct.HlsClient.html)
- [transmux and StreamingTsDemux](https://docs.rs/transmux/latest/transmux/)
- [container-probe](https://docs.rs/container-probe/latest/container_probe/)
- [Reqwest](https://docs.rs/reqwest/latest/reqwest/)
- [Tauri HTTP plugin and Reqwest relationship](https://docs.rs/tauri-plugin-http/latest/tauri_plugin_http/)
- [Rubato](https://docs.rs/rubato/latest/rubato/)
- [CPAL](https://docs.rs/cpal/latest/cpal/)
- [stream-download, optional rather than baseline](https://docs.rs/stream-download/latest/stream_download/)

Some hls-runtime subpages were intermittently unavailable during research. Its top-level documentation establishes the advertised architecture, not a completed source audit. Milestone 0 must inspect the actual published implementation before coding the adapter.
