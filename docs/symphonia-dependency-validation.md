# Symphonia migration: dependency validation

Required by plan section 5. Findings below come from reading the vendored source of each
pinned release and, where noted, from running code against it. No credentials appear here.

Toolchain: Rust stable 1.98.1. Workspace MSRV raised 1.77.2 -> 1.95 (hls-runtime requirement).
CI already used `dtolnay/rust-toolchain@stable`, so the previous 1.77.2 claim was inaccurate.

## Selected versions

| Crate | Version | Features | Notes |
| --- | --- | --- | --- |
| hls-runtime | =0.6.0 | default-features = false | Caller-driven client; requires Rust 1.95 |
| transmux | =0.24.0 | default-features = false | Shared with hls-runtime; no duplicate version |
| container-probe | =0.1.0 | default-features = false | Bounded prefix detection |
| symphonia | =0.6.0 | aac, mp3 | Pulls symphonia-codec-aac 0.6.1, bundle-mp3 0.6.1 |
| broadcast-common | =9.3.0 | dev-dependency only | Used by fixture/parity tests |
| reqwest | =0.12.28 | json, rustls-tls, stream | Already pinned in the app |
| cpal | =0.18.1 | — | Currently Windows-only; must be promoted to all desktop targets |
| rubato | not yet added | — | M3 dependency |

`cargo tree -d` shows no duplicate transmux, confirming the direct-TS and HLS paths share one
demux implementation as the plan requires.

## hls-runtime 0.6.0

Caller-driven core, exactly as the plan wants: the library owns no HTTP stack. `HlsClient::new(url)`
then `poll() -> Option<Action>`, `on_playlist(bytes)`, `on_resource(id, bytes)`, `on_error(id)`,
`next_output() -> Option<Output>`.

`Action`: `FetchPlaylist { url, blocking, skip }`, `FetchResource { id, url, byte_range }`, `WaitMs(u64)`.
`Output`: `Init(Vec<u8>)`, `Samples { track_id, samples }`, `Discontinuity`, `EndOfStream`.

**Supported (read in source):** media playlists; `EXT-X-BYTERANGE` byte ranges; `EXT-X-MAP`
initialization sections, re-emitted only when the map changes; `EXT-X-DISCONTINUITY` forwarded
verbatim; `EXT-X-ENDLIST`; blocking playlist reload and delta updates (RFC 8216bis) with graceful
fallback to `WaitMs` when the origin does not advertise them; URL resolution against the playlist URL.

`Output::Samples` carries demuxed compressed access units produced by transmux, preceded by
`Output::Init` containing MP4 initialization bytes. Init is synthesized even for TS segments. The
decoder bridge must therefore parse initialization metadata once and never re-demux sample payloads.
This confirms the plan's "demux exactly once" invariant is achievable as designed.

**NOT supported — two material gaps:**

1. **Master playlists.** Verified empirically, not inferred. Feeding a `#EXT-X-STREAM-INF` master
   playlist to `on_playlist` returns
   `Err(PlaylistParse { line_no: 4, reason: "media segment URI with no preceding #EXTINF" })`,
   after which `poll()` and `next_output()` both return `None` permanently. The client is dead, not
   degraded. The only `EXT-X-STREAM-INF` handling in the crate is in its `server` module, which is
   irrelevant here. Mitigation is straightforward and does not require forking: detect
   `#EXT-X-STREAM-INF` in the fetched body before handing it to `HlsClient`, select a rendition
   deterministically, resolve its URI against the effective post-redirect URL, and construct the
   client against that media playlist.

2. **Encryption.** No `EXT-X-KEY`, AES, or decryption code exists anywhere in the crate. Recognising
   an encryption tag is not decryption support, and here not even the tag is handled. An encrypted
   playlist cannot play. **Owner decision (2026-09-10):** do not implement decryption. Detect
   `EXT-X-KEY` before handing the playlist to `HlsClient` and raise a specific, actionable error, so
   the failure is never a confusing parse error.

**Untested:** live sliding-window behaviour against a real origin, redirects, and gap/`EXT-X-GAP`
tags. These need fixtures and are M2 work.

## container-probe 0.1.0

`probe(&[u8]) -> Probe` and `probe_with_budget(&[u8], budget)`. `DEFAULT_BUDGET` is 64 KiB.

`Probe` is exactly the shape the plan asks for: `Identified { format, confidence, detail }`,
`Ambiguous { candidates }`, `Insufficient { need_at_least }`, and `Unknown`. The distinction between
`Insufficient` (more bytes may help) and `Unknown` (they will not) is the incomplete-versus-
unsupported signal the plan requires, and `need_at_least` bounds the probe directly.

TS stride detection is present and reports its measurement: `Detail::Ts { stride, phase }` covers the
188/192/204/208-byte layouts named in the plan, via a sync lattice rather than a single magic byte,
which is what makes false sync bytes survivable. Relevant `Format` variants for this project are
`MpegTs`, `AdtsAac`, `Mp3`, and `Isobmff`.

No competing detector should be written. This crate covers the plan's detection requirements.

## Symphonia 0.6.0

**AAC: LC only.** In `symphonia-codec-aac-0.6.1`, `decode_inner` matches `self.asc.object_type` and
handles `AudioObjectType::Lc`; every other object type returns `unsupported_error("aac: object type")`.
HE-AAC (SBR) and HE-AACv2 (PS) will therefore fail to decode, as will Main, SSR, and LTP. Coupling
channel elements and program config elements are also explicitly unsupported.

The one station probed live is AAC-LC 44100 Hz stereo ~258 kbps, which decodes correctly.

**Owner decision (2026-09-10):** the lineup is entirely AAC-LC with no variance, so no provider
probing was performed and no HE-AAC handling is being built. Non-LC object types must still surface
as an actionable source error rather than silence, so that a future provider change is diagnosable
immediately. Recording the source fact for whoever reads this later: the LC-only restriction is a
hard match in `decode_inner`, not a soft limitation.

MP3 is provided by `symphonia-bundle-mp3` and decodes the existing fixture correctly.

The decoder is driven through `AudioCodecParameters` with `CODEC_ID_AAC` plus the AudioSpecificConfig
as extra data, or `CODEC_ID_MP3`. AAC access units arriving from transmux are raw, so no ADTS wrapper
should be prepended and no header stripped twice. Confirmed by the passing parity test.

## transmux 0.24.0

`StreamingTsDemux` (incremental, correct for endless sources) and `Fmp4Demux` (used for
`Output::Init`). `DemuxEvent::{TrackAdded, Sample, Discontinuity, ..}`, with `TrackSpec` carrying
`track_id`, `timescale`, and `CodecConfig::{Aac { esds, sample_rate }, MpegAudio { layer, sample_rate }}`.
`Sample` carries `data`, `pts`, and `duration`, all optional timing.

Proven working: arbitrary chunk boundaries (1, 187, 188, 1024 bytes and a 137-byte stride across
segment data) produce identical PCM, so partial TS/PES/audio state survives network chunking.

## Evidence

`cargo test -p apogee-playback-core`: 4 passed, 0 failed, stable across three runs.
`cargo clippy --workspace --all-targets`: clean. `cargo check --workspace`: clean.
The parity test decodes the same audio as continuous TS and as six frame-aligned HLS segments and
asserts sample-for-sample equality: 130 access units, 266240 interleaved f32 samples, one `Output::Init`.

## CPAL 0.18.1 — promoted to all desktop targets, enumeration verified on real hardware

Compiles and enumerates on this Linux box after installing `libasound2-dev`. Live output:

```
host: Alsa
DEFAULT id="alsa:default"  name="Default Audio Device"
        SupportedStreamConfig { channels: 2, sample_rate: 48000, buffer_size: Range { min: 16, max: 262144 }, sample_format: F32 }
output devices (3):
  alsa:null      Discard all samples (playback) or generate zero samples (capture)
  alsa:pipewire  PipeWire Sound Server
  alsa:default   Default ALSA Output (currently PipeWire Media Server)
```

**API change from older CPAL: `Device::name()` is gone.** 0.18 provides `DeviceTrait::description()
-> DeviceDescription` (name, manufacturer, driver, device_type, interface_type, direction, address)
and a separate `DeviceTrait::id() -> DeviceId`.

**Persistence is explicitly supported**, answering the plan's open question. `DeviceId` is
`(HostId, Box<str>)` and its own documentation states that application code should obtain ids via
`DeviceTrait::id` and persist them through `Display`/`FromStr`. So the saved-device setting should
store the `DeviceId` string, not the display name, which the plan correctly warns is not unique.
The structured `DeviceDescription` additionally gives a sound basis for migrating legacy MPV device
selections by descriptive match.

**The default device wants 48 kHz; the provider streams 44.1 kHz.** Resampling is therefore on the
normal path for this machine, not an edge case, and will be exercised in ordinary use. The device
also reports `F32` natively, so no integer sample conversion is needed here, though other devices may
differ and the conversion must stay general.

## Outstanding

- Rubato 5.0.0 selected; its API differs substantially from 4.x (`Async::new_poly`, and an
  `audioadapter` buffer abstraction rather than plain `Vec<Vec<T>>`). Implementation in progress.
- No physical audio has been produced by this pipeline on any platform.
