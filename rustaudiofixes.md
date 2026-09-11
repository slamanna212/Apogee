# Rust audio engine fixes

Prepared 2026-09-10 following review of the seven commits `8c476bc` through `6f0082b`.

This is an implementation plan, not a report of completed fixes. It covers all ten review findings, including the AppImage workaround for issue #67. The objective is to finish the Rust engine integration, preserve the intended playback behavior, and produce correctly signed Linux update artifacts.

## Scope and implementation rules

- Keep the existing Symphonia, transmux, hls-runtime, Rubato, CPAL, and Reqwest architecture. Do not restore MPV or introduce another playback backend.
- Read repository instructions and inspect the current working tree before editing. Preserve unrelated changes and existing release workflow behavior.
- Implement locally and validate each milestone. This plan does not authorize publishing releases, uploading assets, deploying, or using live provider credentials that are not already authorized.
- Keep the audio callback bounded and nonblocking: no allocation, locks, logging, Tauri calls, network work, or FFT processing.
- Preserve session generation and snapshot revision guards. Stop and station changes must cancel old work; late completions must not restart playback or modify current state.
- Verify APIs against the installed versions and relevant official source before choosing packaging hooks or dependency APIs. Do not assume a hook exists from its name or from documentation for another version.
- Add behavioral regression tests at the integration boundaries that failed. Passing isolated controller or DSP tests alone is insufficient.
- Update `docs/symphonia-migration-progress.md`, `docs/symphonia-platform-acceptance.md`, and `TESTING_NEEDED.md` to reflect actual results. Remove stale implementation claims and distinguish automated, silent-device, and audible hardware validation.

## Review baseline

The review ran 186 frontend tests, 77 playback-core tests, and 70 application Rust tests successfully. Application tests required local socket access outside the sandbox. No Windows/macOS execution or live listening was performed during review.

A temporary harness also reproduced two source-boundary defects:

- A first HTTP chunk containing only `#EXTM3U\n` was classified as a media playlist; adding the master tags in the next chunk changed the classification to master. The application commits to the first classification.
- A 93,060-byte TS chunk was successfully identified, but only 65,536 bytes were retained for replay.

## Work order

1. **M1 — Source detection and byte preservation:** findings 8 and 9.
2. **M2 — Session deadlines and recovery:** findings 2, 3, and 4.
3. **M3 — Output buffering, controls, and format changes:** findings 5, 6, and 10.
4. **M4 — Active device switching and default following:** finding 7; builds on M2/M3.
5. **M5 — AppImage construction and signing:** finding 1 and issue #67. This is independent of engine work and may be implemented earlier.
6. **M6 — Integrated acceptance:** combined failure scenarios, native builds, and documented hardware checks.

Keep each milestone buildable and reviewable. Prefer separate commits for distinct behavioral fixes; do not mix packaging changes into DSP changes.

## M1 — Preserve bytes and classify complete playlists

Primary files: `src-tauri/src/playback/source/mod.rs`, `source/http.rs`, `source/hls.rs`, and `src-tauri/playback-core/src/detect.rs`.

### Finding 9: probe overflow loses media bytes

The detector may retain a bounded prefix, but the transport adapter must preserve every consumed byte until it is either replayed into the selected source or rejected explicitly.

Implementation:

1. Make the boundary explicit: either have the detector report how many bytes it consumed, or let the source retain the original chunk and pass only the remaining probe budget into detection.
2. On TS selection, feed both the retained probe and unexamined remainder in their original order, exactly once.
3. On playlist selection, include the remainder in bounded playlist assembly before reading subsequent chunks.
4. Preserve bounded memory. Check limits before extending buffers, and fail explicitly if a playlist exceeds its limit. Do not silently truncate data or turn the probe into an unbounded accumulator.

Regression tests:

- Compare emitted compressed access units and decoded PCM against direct ingestion of the same TS bytes when a chunk crosses the 64 KiB probe boundary.
- Cover one oversized chunk and several chunks where the last crosses the boundary.
- Cover playlist bytes beyond the probe boundary, exact-limit bodies, oversized bodies, and cancellation during assembly.

### Finding 8: master/media routing depends on chunk boundaries

Recognizing the HLS header establishes the container family; it does not establish the playlist subtype or complete encryption policy.

Implementation:

1. After recognizing HLS, read the complete initial playlist under a byte limit, cancellation, and finite deadline.
2. Classify and validate that complete body before selecting the media or master path. Prefer an explicit provisional HLS result if it simplifies the detector contract.
3. Apply the same full-body validation to fetched variants and reloads where relevant, so later encryption tags cannot bypass policy.
4. Retain effective redirect URLs and reuse the initial fetched body. Do not introduce duplicate initial playlist requests.

Regression tests:

- Serve a master playlist split immediately after `#EXTM3U`, across master tag names, and inside attribute lines; assert the selected variant is fetched and decoded.
- Vary all relevant chunk boundaries and compare the result with a single-body response.
- Put encryption tags after the first chunk and assert a clear unsupported-source result.
- Preserve BOM/whitespace handling, redirect resolution, and byte-exact TS replay tests.

Acceptance: source routing and decoded content are invariant under transport chunk boundaries.

## M2 — Enforce deadlines and complete recovery paths

Primary files: `src-tauri/src/network.rs`, `src-tauri/src/playback/engine.rs`, `commands.rs`, and `src-tauri/playback-core/src/session.rs`.

### Finding 2: no enforced connect-to-play deadline

Implementation:

1. Bound the initial request's wait for response headers without imposing a total lifetime timeout on a live stream.
2. Add an attempt-level, monotonic 20-second deadline from starting the attempt until the output callback confirms that it has consumed decoded frames. Successful headers, TS detection, or decoder construction do not satisfy it.
3. Enforce this watchdog independently of incoming data: TS with no supported audio, playlists that never yield samples, and slow trickles must still time out.
4. On timeout, cancel the attempt, release its resources, and report a transient error through the existing controller. Cancel the watchdog after confirmed playback or when the generation is invalidated.
5. Retain stalled-read detection for established streams. Preserve the documented 90-second retry policy unless deliberately revised with tests and rationale; initialize its time accounting at the actual connect phase rather than the first error.
6. Use monotonic elapsed time for retry and health accounting; the current adapter uses wall-clock timestamps. System clock changes must not alter retry budgets.

Regression tests:

- A local server accepts TCP but never sends response headers.
- Headers arrive, followed by regular nonplayable TS data that prevents a read-stall timeout.
- An HLS source stays responsive but never produces playable audio.
- Each case retries or terminates within a test-configured budget; stop and station switch cancel it promptly.
- A valid live stream continues beyond the initial deadline without being terminated.
- Callback confirmation races with timeout: exactly one outcome wins for that attempt.

### Finding 3: full-ring waits conceal output failure

Implementation:

1. Check output health and cancellation while waiting for PCM capacity, and while waiting for decoder input. A stalled producer must not be the only component capable of noticing a device failure.
2. Report a device failure once, invalidate/terminate the failed attempt, and let the controller decide recovery.
3. Audit the bounded event channel at the same boundary. `SyncSender::send` currently blocks a Tokio worker, and session teardown joins the decoder while a network task still owns a sender. Use cancellation-aware async backpressure and a decoder wakeup mechanism that does not depend on a blocked runtime task completing.
4. Keep teardown outside the player-state mutex and ensure it releases the old network connection and output before opening replacements.

Regression tests:

- Use an injected output consumer to fill the ring, stop consumption, and signal device failure. Assert recovery is reached without user intervention.
- Signal output failure while the decoder is waiting for input.
- Stop/switch while the PCM ring is full, event queue is full, or input is empty. Assert bounded teardown and no stale events/audio.
- Exercise a constrained Tokio runtime so blocking-channel mistakes cannot be hidden by spare workers.

### Finding 4: retry startup errors only get logged

Implementation:

1. Route failed `start_session` calls through the controller with the current attempt identity. Do this for both initial startup and retry startup.
2. Retry recoverable device-unavailable errors within the existing budget; surface a terminal failed snapshot once exhausted. Do not leave recovering/connecting state without live work or a scheduled retry.
3. Ensure only one completion and one retry can be accepted per attempt. A station generation alone may need an internal attempt identifier to reject late watchdog or output failures from a previous retry.
4. Keep frontend invoke errors consistent with backend snapshots and prevent stale command failures from replacing a newer session's state.

Regression tests:

- Inject startup failures on the first attempt and on a scheduled retry.
- Assert bounded retries, a final error snapshot, and successful recovery if a device becomes available within the budget.
- Switch stations during the retry delay and during startup; the old attempt must not install a session or affect new playback.

Acceptance: every connecting/recovering state has bounded active work; every failure can terminate or recover without user intervention.

## M3 — Wire buffering and controls to real consumption

Primary files: `src-tauri/src/playback/engine.rs`, `audio_out.rs`, and `src-tauri/playback-core/src/output.rs`, `dsp.rs`, `pipeline.rs`.

### Finding 5: BufferGate does not control consumption

Implementation:

1. Make the output consumer honor the buffering state. While buffering, emit silence without draining queued PCM until the configured start threshold is reached.
2. Apply the lower rebuffer threshold with hysteresis, then accumulate back to the start threshold before resuming. Use frame counts in the actual output format.
3. Propagate buffering and resumed-consumption transitions to the controller outside the callback. Replace the one-way `announced_playing` behavior so multiple buffering/recovery cycles work.
4. Keep the callback's transition reporting allocation-free and nonblocking, using atomics or an appropriately bounded mechanism.
5. Reset sustained-playback health tracking on starvation. UI status, OS playback state, presence, and scrobbling must stop treating prolonged silence as healthy playback.

Regression tests:

- Feed less than the start threshold and assert no queued audio is consumed.
- Cross the threshold and assert playback begins only after consumption.
- Starve, refill partially, then refill fully; assert correct silence, preserved queued frames, and exactly the expected state transitions.
- Test repeated cycles and jitter around thresholds without state flapping.

### Finding 6: gain is baked into buffered PCM

Implementation:

1. Move volume/mute application to the consumption side of the PCM ring, with a short ramp and callback-safe setting delivery.
2. Follow the original plan for EQ: prepare coefficients off the callback and apply them with preallocated per-channel state near consumption. Preserve headroom and output protection; do not apply gain or EQ twice.
3. Keep disabled/flat EQ behavior, volume persistence, mute state, and sample-format conversion semantics intact. Derive ramp duration from the actual output sample rate.
4. Move the visualizer tap to the corresponding consumed, processed samples. Feed an independent FFT worker through a bounded nonblocking queue; drop analysis data rather than blocking audio.

Regression tests:

- Prefill a two-second ring, change mute/volume, and assert the next callbacks respond within the configured ramp rather than after draining the ring.
- Test rapid settings changes, muted volume changes, unmute, clipping protection, and supported output sample formats.
- Verify no callback allocations or blocking operations are introduced.
- Verify visualizer levels correspond to consumed post-control audio and disabling it stops analysis work.

### Finding 10: channel changes are missed when rate is unchanged

Implementation:

1. Track the source format as at least `(sample_rate, channels)` and rebuild conversion whenever either changes.
2. Reset or replace pending resampler state at the format boundary so old-layout samples cannot mix with new-layout samples.
3. Define discontinuity handling explicitly: preserve continuity across ordinary HLS segments, but reset affected decoder/conversion/DSP state for real format changes. Keep old-format queued audio correctly framed or discard it through an explicit discontinuity operation.
4. Publish the new format and keep bitrate accounting valid across sample-rate changes.

Regression tests:

- Change mono to stereo and stereo to mono at an unchanged rate. Use distinct per-channel signals and verify frame counts, duration, and channel mapping.
- Change rate with and without a channel change, including pending partial resampler input.
- Preserve continuous-TS/HLS PCM equivalence and no-reset-on-normal-segment tests.

Acceptance: buffering governs audible output; controls affect already-buffered playback promptly; format transitions preserve frame interpretation.

## M4 — Switch active devices and follow the default

Primary files: `src-tauri/src/playback/commands.rs`, `device.rs`, `audio_out.rs`, `engine.rs`, `src/pages/Settings.tsx`, and the player client/store tests.

### Finding 7: device selection only affects future sessions

Implementation:

1. Make `player_set_device` trigger an active output reconfiguration when playing. Reuse the lifecycle and failure handling established in M2.
2. Prefer retaining the network source when a safe output-only rebuild is practical. If session restart is required, cancel and close the old session before reconnecting so provider connection limits are respected; document the interruption.
3. Construct the ring, resampler, and callback from the same resolved output configuration. Avoid independently resolving the default twice, which can produce mismatched formats if it changes between calls.
4. Detect default-device changes using supported native notifications or bounded polling outside the callback. Reopen only when the effective default changes and the user selected system default.
5. Preserve a specifically selected device preference when unplugged. Report the actual fallback device separately; document whether the preferred device is restored automatically when it returns.
6. Handle open failures visibly rather than swallowing them in Settings. Keep the picker and diagnostics consistent with requested versus effective output.

Regression tests:

- Switch between injected devices with different rates/channel counts while playing; verify one active output owner, correct conversion, and no old callback access after teardown.
- Change the default while following it, and while explicitly selecting a device; only the former should follow the change.
- Remove the selected device, remove all devices, and restore a device; assert fallback/retry/failure behavior without losing preferences.
- Race device changes with stop, station switch, and retry. Only the latest accepted intent may install output.
- Retain legacy MPV identifier migration tests.

Hardware acceptance: check live selection, USB/Bluetooth unplug/replug where available, and system-default changes. Record Linux results separately from pending Windows/macOS validation.

## M5 — Replace the issue #67 packaging workaround

Primary files: `.github/workflows/release.yml`, `.github/workflows/dev-prerelease.yml`, `scripts/fix-appimage-host-libs.sh`, and Linux Tauri bundle configuration.

### Finding 1: the uploaded AppImage is mutated after signing

Required order:

```text
compile → stage AppDir → apply host-library policy → build final AppImage
        → sign final bytes → generate updater metadata → verify → upload
```

Implementation:

1. Inspect the actual pinned Tauri CLI/bundler, linuxdeploy integration, and tauri-action behavior. Confirm which file is signed and which file the Linux entry in `latest.json` references.
2. Prefer a supported bundle-time exclusion or an AppDir hook that excludes the conflicting `libwayland-client.so.0` before final AppImage construction. Verify the hook executes at the required stage. A generic before-build hook is not sufficient if linuxdeploy copies the library back afterward.
3. If the installed tooling has no supported pre-sign exclusion mechanism, use an explicit Linux artifact pipeline: build/stage locally, finalize the AppImage, sign that final file using Tauri tooling, and generate metadata from its signature. Separate build/finalization from upload rather than replacing already-uploaded assets.
4. Any necessary repack must be a deterministic build step before signing. Pin the repacking tool and verify its checksum; remove the floating `continuous` tool download.
5. Do not exclude unrelated libraries speculatively. Verify the resulting loader resolves the intended host Wayland library on the affected distro and on the supported baseline distro.
6. Share the packaging implementation between stable and prerelease workflows. Preserve tag-specific download URLs, the existing release ID, version synchronization, and other platforms' updater entries. Avoid concurrent manifest writes that can lose entries.
7. Remove both post-upload AppImage replacement steps. Remove the old script if superseded, or rename/refactor it to accurately describe its pre-sign build role.
8. Produce checksums after finalization. No signed artifact may be modified after signature/manifest generation, including by later asset-alias steps.

Automated acceptance:

- Using a disposable signing key, verify the final AppImage against the exact signature embedded in the generated manifest. Mutating one byte must fail verification.
- Extract the final AppImage and assert the excluded library is absent while required application libraries remain present.
- Validate stable and prerelease manifests: correct tag URLs, asset names, signatures, and preservation of all platform entries.
- Cover the no-library-to-remove case as well as the issue #67 case.
- Keep these checks local or in artifact validation jobs; do not publish a release merely to test the fix.

Manual acceptance:

- Launch the finalized AppImage on the supported baseline Linux system and the affected rolling-release environment where available.
- Exercise an update from an installed older build to the finalized artifact using a controlled test setup. Verify signature acceptance, installation, relaunch, and audible playback.

Acceptance: issue #67 remains fixed, and the distributable AppImage and updater signature describe exactly the same bytes.

## M6 — Final integrated validation

Run the existing suites plus the new integration regressions:

```bash
npm test
npm run lint
npm run build
cd src-tauri
cargo test -p apogee-playback-core --offline
cargo test --lib --offline
cargo check --offline
```

Use the repository's supported Node/Rust toolchains. Offline Cargo commands require cached dependencies; install missing dependencies through the normal authorized workflow if necessary. Local HTTP tests require permission to bind loopback sockets. Report skipped hardware tests explicitly rather than counting early-return tests as device validation.

Combined acceptance scenarios:

- Direct TS and HLS: tune, reach confirmed playback, mute, change volume/EQ, stop, and retune.
- Slow startup and starvation: enforce connect deadlines, rebuffer correctly, recover within the retry budget, and fail visibly when exhausted.
- Rapid station and device changes during full queues, empty queues, startup, timeout, and retries: no deadlocks, leaked sessions, old audio, or stale UI state.
- Output loss and restoration: no indefinite full-ring waits or permanent loading states.
- Format changes: mono/stereo and rate transitions preserve timing and channel layout.
- Long playback and suspend/resume: bounded memory, continued controls, and eventual recovery or a clear failure.
- Signed Linux artifact: issue #67 startup check and controlled update/install/relaunch.

Build Windows and macOS targets on native runners. Physical playback, hotplug, default following, and update behavior remain pending until actually tested on those systems; a cross-platform build is not hardware acceptance.

## Completion checklist

- [x] Finding 1: final AppImage is signed only after all modifications; updater metadata uses that signature. Disposable-key artifact verification remains a CI/manual acceptance check.
- [x] Finding 2: initial headers and actual connect-to-play are bounded and cancellable.
- [x] Finding 3: device failure escapes producer waits and triggers recovery.
- [x] Finding 4: startup failures drive retries or terminal snapshots.
- [x] Finding 5: callback consumption honors startup/rebuffer thresholds and reports transitions.
- [x] Finding 6: mute/volume affect queued playback promptly; analysis does not block playback.
- [x] Finding 7: active device selection and system-default following are implemented. Physical hotplug/default-change validation remains pending.
- [x] Finding 8: complete playlists determine HLS subtype regardless of chunking.
- [x] Finding 9: all consumed probe bytes are replayed exactly once or rejected explicitly.
- [x] Finding 10: rate/channel changes rebuild conversion safely.
- [x] Regression suites pass and native/hardware limitations are documented.
- [x] Migration progress and manual-test documentation match the final implementation.
- [x] No release was published or external assets changed as part of local implementation.

Implementation validation on 2026-09-11: 187 frontend tests and 195 Rust workspace tests passed;
frontend build and Rust check passed; Clippy passed with warnings denied. Frontend lint retained only
the two pre-existing `ChannelCard.tsx` fast-refresh warnings. Shell syntax and workflow YAML were
validated locally. The AppImage finalization/signature/update flow still needs its documented
disposable-key artifact run: a local unsigned AppImage build compiled successfully but Tauri timed
out downloading its Linux bundler helpers before producing the artifact. Physical device/platform checks remain pending as recorded in
`TESTING_NEEDED.md` and `docs/symphonia-platform-acceptance.md`.
