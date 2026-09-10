# Symphonia platform acceptance

Created during M0. Updated after milestone scoping. No physical audio test has been run yet.

**Owner decision (2026-09-10):** the user has Linux hardware only. Windows, macOS Apple Silicon, and macOS Intel have no tester or device and are expected to stay PENDING MANUAL VALIDATION through M5. The user has chosen to proceed through the full plan, including MPV removal, with those three platforms unvalidated. That is an accepted, explicitly recorded release-readiness blocker, not a passed check. No release may be published on this basis without a further explicit decision.

| Platform | Build / headless evidence | Manual tester/device | Hardware status |
| --- | --- | --- | --- |
| Linux desktop | Local checks green; release runner rapture-apogee | slamanna212, this dev box | **AUDIBLE PLAYBACK CONFIRMED 2026-09-10** |
| Windows x64 | Hosted native build configured; no migration build tested | UNASSIGNED | PENDING MANUAL VALIDATION |
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

Not a defect: one station (stream 1124, PopRocks) returns HTTP 503 from Dispatcharr. Plain `curl`
reproduces the identical 503 with Apogee uninvolved, so that channel's upstream is unavailable rather
than the engine misbehaving.

Checkpoints:

- M3: audible direct TS/HLS, stop/switch, device enumeration.
- M4: hotplug, default-device following, legacy MPV device migration, EQ/mute/visualizer, media keys.
- M5: final bundle clean install, two-hour session, suspend/network recovery, update/install/relaunch.

For every check record date, tester, OS/architecture, audio device/backend, exact artifact, steps, outcome and sanitized diagnostics. Build, headless and manual results must remain separate. Missing owners/hardware block platform release readiness, not independent implementation.
