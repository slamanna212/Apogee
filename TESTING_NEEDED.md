# Testing needed

Things I changed that I could not verify myself, in priority order. Anything already
covered by an automated test is not listed here.

See also `docs/symphonia-manual-test-guide.md` for the audio-engine checks, and
`docs/symphonia-platform-acceptance.md` for the per-platform matrix.

---

(entries appended below as they happen)
## 0. Corrective audio and AppImage acceptance (2026-09-11)

The corrective plan is implemented and covered by automated Rust tests, but these checks require
real hardware or a release-runner artifact:

- While two seconds of PCM are queued, change volume, mute, and EQ; the next callback should ramp to
  the new setting rather than waiting for the queue to drain.
- Force a network starvation and recovery. Audio should remain silent while refilling, preserve the
  queued tail, and return the UI/media-session state to playing only when consumption resumes.
- Switch outputs while playing, change the system default while following it, then repeat with a
  specific device selected. Only system-default mode should follow the OS change.
- Unplug/replug a specifically selected USB/Bluetooth device. Its saved preference must survive;
  fallback/recovery or a visible failure is acceptable, silence with a loading state is not. A
  healthy fallback stream intentionally stays on the fallback until the device is selected again;
  a later session or retry resolves the preserved preference.
- Run stable and prerelease Linux packaging with a disposable updater key. Extract the final
  AppImage and confirm `libwayland-client.so.0` is absent, verify the final file against the exact
  signature stored in `latest.json`, mutate one byte and confirm verification fails, then exercise a
  controlled update/install/relaunch. No release needs to be published for this test.

## 1. Channel loading and metadata (highest risk)

The Xtream and StellarTunerLog clients moved from TypeScript into Rust. Their unit tests
cover URL building, credential escaping and error redaction, but **nothing has run against
your real server** since the change.

- Open the app and confirm the channel list loads.
- Confirm now-playing metadata appears and updates.
- Open a channel's play history (needs a StellarTunerLog API key configured). Confirm it
  loads, and that with **no** key configured you still get now-playing rather than an error:
  `/nowplaying` and `/channels` are keyless, only `/history` needs a key.
- In Settings, change the category selection and confirm the list reloads.
- Break the Xtream password deliberately and confirm the error names the failure without
  printing your password, then fix it and confirm recovery.

## 2. Windows update flow (cannot be tested on Linux at all)

The Job Object was removed along with MPV, and with it the `CREATE_BREAKAWAY_FROM_JOB` flag
the updater passed when launching the installer. The custom installer path was **kept**,
because it also reports launch failures that the plugin's own path swallows.

- Build a dev release, install it on Windows, then update to a newer dev build.
- Confirm the installer window appears and survives Apogee exiting.
- Confirm the app relaunches afterwards.
- This is the single riskiest change in the migration: a broken updater blocks its own fix.

## 3. Notification artwork

Artwork downloads moved onto the shared HTTP service. The 3 MiB cap was preserved and is
asserted by a test, but the fetch path itself is new.

- Trigger a track-change notification and confirm the artwork still appears.
- Confirm repeat notifications for the same track use the cache rather than refetching.

## 4. Last.fm scrobbling

Last.fm moved off its own HTTP client onto the shared service, including its form POST.

- Connect a Last.fm account, play a station, and confirm now-playing updates and a scrobble.
- Confirm an authentication failure still produces a readable error.

## 5. Update check

The GitHub release list is fetched in Rust now.

- Confirm the update check still finds releases and the changelog renders.
- Confirm the beta channel still includes prereleases and the stable channel does not.

## 6. Linux CI runner prerequisite

`libasound2-dev` was added to the GitHub-hosted Linux dependency step. The self-hosted
`rapture-apogee` runner **skips that step** (it is gated on `runner.environment ==
'github-hosted'`), so that machine needs the package installed by hand or the Linux build
will fail at the CPAL crate.

## 7. Packaging

`deb.depends` and `rpm.depends` changed from `mpv` to `libasound2` / `alsa-lib`.

- Build a `.deb` and an `.rpm` and confirm they install on a machine without mpv.
- Confirm audio works on that machine, which is the real proof the runtime dependency is
  declared correctly.

## 8. Everything in the audio test guide

`docs/symphonia-manual-test-guide.md` still has outstanding items: rapid station switching,
stop latency, pushing the equaliser hard, the spectrum display across several stations,
device switching and hotplug, suspend/resume, and a long session.
