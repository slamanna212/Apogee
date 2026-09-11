# Symphonia platform acceptance

Created during M0. Last implementation update: 2026-09-11. Linux physical playback was confirmed on
2026-09-10; no new physical-device run was performed for the corrective buffering/control/device
work, so the checks listed below remain pending.

**Owner decision (2026-09-10):** the user has Linux hardware only. Windows, macOS Apple Silicon, and macOS Intel have no tester or device and are expected to stay PENDING MANUAL VALIDATION through M5. The user has chosen to proceed through the full plan, including MPV removal, with those three platforms unvalidated. That is an accepted, explicitly recorded release-readiness blocker, not a passed check. No release may be published on this basis without a further explicit decision.

| Platform | Build / headless evidence | Manual tester/device | Hardware status |
| --- | --- | --- | --- |
| Linux desktop | Local checks green; release runner rapture-apogee | slamanna212, this dev box | **AUDIBLE PLAYBACK CONFIRMED 2026-09-10** |
| Windows x64 | Hosted native build configured; not yet built since MPV removal | User has a Windows machine; will test dev builds before release | PENDING MANUAL VALIDATION — **includes the updater**, see TESTING_NEEDED.md |
| macOS Apple Silicon | Hosted universal build configured; no migration build tested | UNASSIGNED | PENDING MANUAL VALIDATION |
| macOS Intel | Universal slice does not prove execution | UNASSIGNED | PENDING MANUAL VALIDATION |

Artifact handoff: produce local/native test bundles with the existing build configuration; record commit plus diff/artifact hash if uncommitted. Supply artifact and test instructions to a tester before M3 acceptance. Do not publish a release automatically.

## First audible playback (2026-09-10)

Linux dev box, ALSA via PipeWire, device `alsa:default` at 48 kHz F32, decoding a 44.1 kHz AAC-LC
station, so the resampler was on the live path. Backend is Dispatcharr over plain HTTP on the LAN.

```
playback started for channel Unwell Music
heartbeat: Unwell Music playing, bitrate=260kbps
heartbeat: Unwell Music playing, bitrate=261kbps
```

The reported 260-261 kbps is the AAC payload rate rather than transport throughput, confirming the
bitrate accounting against a live stream rather than only fixtures. Stop and re-tune both worked.

This satisfies the M3 checkpoint's audible-output requirement for Linux only. The remaining M3/M4
manual checks (device hotplug, default-device following, long session, suspend/resume) are still
outstanding, and Windows and macOS remain PENDING MANUAL VALIDATION with no tester or device.

The 2026-09-11 corrective pass changed when queued PCM is consumed and controlled, and made device
selection active. Recheck volume/mute response with a prefilled buffer, repeated starvation/refill,
live device switching, unplug/replug, and changing the system default before release. Automated and
silent-device tests do not count as those audible checks.

Not a defect: one station (stream 1124, PopRocks) returns HTTP 503 from Dispatcharr. Plain `curl`
reproduces the identical 503 with Apogee uninvolved, so that channel's upstream is unavailable rather
than the engine misbehaving.

Checkpoints:

- M3: audible direct TS/HLS, stop/switch, device enumeration.
- M4: hotplug, default-device following, legacy MPV device migration, EQ/mute/visualizer, media keys.
- M5: final bundle clean install, two-hour session, suspend/network recovery, update/install/relaunch.

For every check record date, tester, OS/architecture, audio device/backend, exact artifact, steps, outcome and sanitized diagnostics. Build, headless and manual results must remain separate. Missing owners/hardware block platform release readiness, not independent implementation.
