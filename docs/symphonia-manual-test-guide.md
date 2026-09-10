# Manual test guide: the new audio engine

For the Linux desktop acceptance pass. Windows and macOS remain PENDING MANUAL VALIDATION
with no tester or device assigned; see `symphonia-platform-acceptance.md`.

## Before you start

- `libasound2-dev` must be installed (build-time only; end users need just the runtime library).
- MPV is still installed, still registered, and still works. Nothing has been removed. If the new
  engine misbehaves, the old path is intact and every change is uncommitted.
- Run the app the way you normally do. Nothing here asks you to run a build command.

## What changed that you can feel

Playback no longer spawns an external process. Audio is decoded in-process and written straight to
your sound card. Your default device reports 48 kHz and the stations are 44.1 kHz, so resampling is
active on the normal path, not an edge case.

## The checks that matter most

Work down this list. Stop and report at the first thing that is wrong; later checks assume earlier
ones passed.

1. **Sound comes out.** Select a station. You should hear audio within a few seconds. The status
   should move through loading to playing, and it should flip to playing only when audio is genuinely
   audible, not the moment the connection succeeds.
2. **The right station plays.** Switch stations several times in a row, quickly. The station you land
   on last must be the one you hear. No fragment of the previous station should leak through after
   the switch.
3. **Stop is prompt.** Press stop while playing. Audio should cease within roughly a quarter second,
   allowing for device buffering.
4. **Volume and mute.** Drag the volume slider. It should change smoothly with no clicks or steps.
   Mute should silence output and unmute should return to the same level, not to full volume.
5. **The equaliser.** Enable it and move a band. The change should be audible and should not click.
   Push several bands to +12 and confirm it does not distort. This case clipped under MPV, so it is
   worth pushing hard.
6. **The spectrum display.** Bars should move with the music and use most of the vertical range. If
   they sit permanently full or permanently dark, the calibration needs adjusting; say which.
   Check it during a quiet passage and with mute engaged.
7. **Device selection.** Open Settings and check the device list. Your previously saved device should
   have carried over, or you should see a message explaining why it could not. Switch devices while
   playing.
8. **Unplug something.** With audio playing through headphones or a USB output, unplug it. The app
   should recover or fail clearly, and your saved preference should not be silently erased.
9. **Bad credentials.** Temporarily break the password in Settings. It should fail quickly with a
   clear message rather than retrying four times, and the message must not contain your password.
10. **A long session.** Leave a station playing for a couple of hours. Watch for memory growth,
    dropouts, or the stream dying without recovering.

## What to record for each result

Date, the station used, the audio device and backend, what you did, what happened. For anything that
fails, the exact on-screen message and whether it reproduced.

## Known limitations, so they are not reported as bugs

- The spectrum calibration was measured from a short sample of a single station. Reasonable across the
  lineup is the goal; perfect is not expected yet.
- Equaliser output is not bit-identical to MPV. Filter shape and headroom match; the sample values do
  not, and parity was never claimed.
- Encrypted HLS playlists are refused with a clear message rather than played. No provider stream is
  known to use encryption.
- Byte-range HLS segments are fetched whole and sliced locally. Correct, just not bandwidth-optimal,
  and no provider playlist reaches that path.
