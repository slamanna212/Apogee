# SXM Tuner — Desktop App Build Spec

Hand this whole document to Claude Code as the starting brief. It covers architecture, exact API contracts, file layout, and build order. Where an external system's exact field names need to be confirmed against a live instance, that's flagged explicitly — don't guess past those points, verify against the real API response first.

## 1. What this is

A cross-platform (Windows + Linux) desktop tuner app:
- Connects to an IPTV backend **via the Xtream Codes API only** (no vendor-specific API, no branding tie to any particular backend)
- Shows channels from one configured category/group, in provider order
- While browsing, each channel row shows its current artist/song if available
- Selecting a channel plays audio and shows artist / title / album / album art from StellarTunerLog, plus live bitrate
- Basic transport controls: play/pause, volume

Not in scope: EPG, recording, video, multi-group browsing, channel search (add later if wanted).

## 2. Stack

- **Shell:** Tauri v2 (Rust core, system webview) — small binaries, native on both target OSes, no Electron/Chromium bundling overhead
- **Frontend:** React + TypeScript + **Mantine UI** (AppShell, ScrollArea, Table/List, Card, Slider for volume)
- **State:** Zustand or plain React context — this app's state is small (channel list, now-playing map, player state), don't reach for Redux
- **Playback:** mpv as a sidecar process, controlled over JSON IPC — handles MPEG-TS/HLS transparently, no need to hand-roll stream demuxing
- **HTTP:** Tauri's `@tauri-apps/plugin-http` `fetch` from the frontend (bypasses CORS in the webview) — no custom Rust HTTP client needed for Xtream/StellarTunerLog calls
- **Settings storage:** `@tauri-apps/plugin-store` for non-secret config (base URL, group id, poll interval); Rust `keyring` crate for the Xtream password and StellarTunerLog API key — don't put credentials in the plaintext store
- **OS media integration:** Rust `souvlaki` crate — pushes now-playing metadata and play/pause state to Windows SMTC and Linux MPRIS (and macOS's Now Playing Center for free, if that target gets added later)

Only Rust code needed: the mpv process lifecycle + IPC bridge (spawn, send commands, read replies/events) exposed via `#[tauri::command]`, the `keyring` read/write commands, and the `souvlaki` media-session bridge. Everything else — API calls, UI, matching logic — is TypeScript.

## 3. Xtream Codes integration

All Xtream Codes API calls hit `player_api.php` with query params. Base URL, username, password are set once in Settings.

**Get categories (for the Settings group picker):**
```
GET {base}/player_api.php?username={user}&password={pass}&action=get_live_categories
```
Response: array of `{ category_id: string, category_name: string, parent_id: number }`. User picks one `category_name` in Settings; store its `category_id`.

**Get channels in the configured group:**
```
GET {base}/player_api.php?username={user}&password={pass}&action=get_live_streams&category_id={category_id}
```
Response: array of channel objects. Known Xtream fields to use:
- `stream_id` — numeric, needed to build the playback URL
- `name` — display name
- `stream_icon` — logo URL
- `num` — provider-assigned channel number; **sort the list by this field ascending** to get "order given from the provider"
- `epg_channel_id` — sometimes present, useful as a secondary match key against StellarTunerLog

**Stream URL:**
```
{base}/live/{user}/{pass}/{stream_id}.ts
```
Currently `.ts` (MPEG-TS) on the live backend, but this isn't guaranteed — some Xtream accounts/providers serve `.m3u8` instead, and it can vary per-channel or per-provider config. Don't hardcode the extension: make it a Settings field defaulting to `.ts`, and — better — have the app detect it automatically per channel (try `.ts`, fall back to `.m3u8` on a failed/empty response, cache the result per `stream_id` so it's not re-detected every play).

No native vendor API calls anywhere in this project — Xtream is the only backend contract.

## 4. StellarTunerLog integration

Base: `https://api.stellartunerlog.com/v1/`, auth via `X-API-Key` header (key entered in Settings, stored via keyring).

**Single polling endpoint, used for everything:**
```
GET /nowplaying
Header: X-API-Key: {key}
```
Returns near-real-time now-playing for all ~437 live channels in one response (artist, title, album, artwork, channel name/number). Poll this **once every 20–30s** while the app is open (regardless of whether audio is playing) — one call updates now-playing for every visible channel row, not just the one playing. Don't call per-channel `/history/{channel_id}` for this; that endpoint is for historical browsing, not live display, and would mean one call per visible channel instead of one call total.

**Matching Xtream channels to StellarTunerLog entries:** match on **channel name** — the Xtream `name` field should equal or closely match StellarTunerLog's channel name. Normalize both sides before comparing (lowercase, strip whitespace/punctuation, strip common suffixes like "HD"/"Radio"/quality tags) and use a fuzzy string match (e.g. Levenshtein/`fuzzysort`) with a similarity threshold, not exact equality, since Xtream panel naming and StellarTunerLog's naming won't be byte-identical. Build the matcher as an isolated, testable module (`src/lib/channelMatcher.ts`) — feed it the real `get_live_streams` names and the real `/nowplaying` names side by side and tune the threshold/normalization from actual output, don't guess it upfront. Channel number is not reliable for this match since Xtream panels can renumber channels independently of SiriusXM's on-air numbering.

Store the last-fetched now-playing response in a map keyed by matched channel id; both the channel list rows and the now-playing panel read from the same map so they never disagree.

## 5. Playback + bitrate (mpv over JSON IPC)

Spawn on channel select:
```
mpv --no-video --idle=yes --input-ipc-server=<path> --volume=<n>
```
- Linux: unix socket path, e.g. `/tmp/sxm-tuner-mpv.sock`
- Windows: named pipe, e.g. `\\.\pipe\sxm-tuner-mpv`

On channel switch, send `loadfile` with the new URL rather than respawning the process:
```json
{ "command": ["loadfile", "<stream_url>", "replace"] }
```

Transport controls map directly to IPC commands:
- Play/pause: `{"command": ["set_property", "pause", true|false]}`
- Volume: `{"command": ["set_property", "volume", 0-100]}`

**Bitrate:** mpv exposes `audio-bitrate` and `packet-audio-bitrate` as properties. Use `observe_property` on `audio-bitrate` at startup so mpv pushes updates as events instead of polling constantly:
```json
{ "command": ["observe_property", 1, "audio-bitrate"] }
```
Events arrive as `{"event": "property-change", "id": 1, "name": "audio-bitrate", "data": <bits/sec>}` — convert to kbps for display. If `audio-bitrate` reads null/0 for a given stream (some containers don't populate it live), fall back to polling `packet-audio-bitrate` on a 2–3s timer as a secondary source.

Rust side needs one long-lived task per mpv session that: owns the socket/pipe connection, writes commands, and forwards IPC replies/events back to the frontend via a Tauri event channel (`app.emit("mpv-event", payload)`). Frontend subscribes with `listen("mpv-event", ...)`.

## 6. OS "Now Playing" integration

Push the same metadata already shown in the app into the OS-level media widgets — Windows System Media Transport Controls (the flyout that shows on the taskbar/lock screen/keyboard media keys), and Linux's MPRIS (surfaces in GNOME/KDE media widgets, `playerctl`, etc.).

Use the **`souvlaki`** Rust crate — it's a single cross-platform abstraction over Windows SMTC, Linux MPRIS (D-Bus), and macOS's Now Playing Center, so one integration point covers all current and possibly-future targets instead of writing three platform-specific implementations.

- `MediaControls::new(config)` — on Windows, `PlatformConfig` needs an `HWND`, which Tauri's window handle can provide (`window.hwnd()` via `raw-window-handle`, or the WebView2 window handle depending on Tauri version — confirm the exact accessor against the Tauri version in use). On Linux, it just needs a `dbus_name`/`display_name`. No config needed on macOS.
- Call `controls.set_metadata(MediaMetadata { title, artist, album, cover_url, .. })` every time the matched now-playing entry for the active channel changes — same event that updates `NowPlayingPanel.tsx` should also fire this.
- Call `controls.set_playback(Playing/Paused)` on every play/pause state change so the OS widget's play/pause button icon stays in sync.
- `controls.attach(|event| ...)` receives `MediaControlEvent::Play`/`Pause`/`Toggle` from the OS side (lock screen, hardware media keys, headset buttons) — wire these back into the same play/pause path as the in-app transport controls, not a separate code path, or the two will drift out of sync.
- This lives in `src-tauri/src/media_session.rs`, driven by a `#[tauri::command]` the frontend calls whenever now-playing/player state changes (mirrors the `mpv.rs` pattern already used for playback).

Since there's no track duration or seek position for a live radio stream, skip `set_playback` position/duration fields entirely — just title/artist/album/art and play/pause state.

## 7. Settings

Fields: Xtream base URL, username, password, stream extension (`.ts`/`.m3u8`), selected category/group, StellarTunerLog API key, StellarTunerLog poll interval (default 25s), default volume.

Flow: "Test Connection" button calls `get_live_categories`, populates a dropdown to pick the group, saves `category_id` — don't make the user type a raw category id.

## 8. Suggested file structure

```
sxm-tuner/
├── src-tauri/
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── capabilities/
│   │   └── default.json          # http, shell, store, keyring permissions
│   └── src/
│       ├── main.rs
│       ├── mpv.rs                # process spawn + IPC socket/pipe bridge
│       ├── secrets.rs            # keyring read/write commands
│       └── media_session.rs      # souvlaki bridge — OS now-playing widgets
├── src/
│   ├── main.tsx
│   ├── App.tsx                   # Mantine AppShell layout
│   ├── lib/
│   │   ├── xtream.ts             # get_live_categories / get_live_streams / stream URL builder
│   │   ├── stellarTunerLog.ts    # /nowplaying poller
│   │   ├── channelMatcher.ts     # Xtream <-> StellarTunerLog matching, isolated + testable
│   │   └── mpvClient.ts          # thin wrapper around the Tauri mpv commands/events
│   ├── stores/
│   │   ├── settingsStore.ts
│   │   ├── channelStore.ts       # channel list + now-playing map
│   │   └── playerStore.ts        # current channel, pause state, volume, bitrate
│   ├── components/
│   │   ├── ChannelList.tsx       # Mantine ScrollArea + rows
│   │   ├── ChannelRow.tsx        # logo, number, name, live artist/title subtitle
│   │   ├── NowPlayingPanel.tsx   # art, artist, title, album, bitrate badge
│   │   ├── TransportControls.tsx # play/pause, volume slider
│   │   └── SettingsModal.tsx
│   └── types/
│       ├── xtream.ts             # XtreamCategory, XtreamChannel
│       ├── stellarTunerLog.ts    # NowPlayingEntry
│       └── player.ts             # PlayerState
└── package.json
```

## 9. Core TypeScript types

```typescript
// types/xtream.ts
interface XtreamCategory {
  category_id: string;
  category_name: string;
}

interface XtreamChannel {
  stream_id: number;
  name: string;
  stream_icon: string;
  num: number;
  epg_channel_id?: string;
}

// types/stellarTunerLog.ts
interface NowPlayingEntry {
  channelNumber: number;
  channelName: string;
  artist: string;
  title: string;
  album?: string;
  artworkUrl?: string;
  updatedAt: string;
}

// types/player.ts
interface PlayerState {
  status: "idle" | "loading" | "playing" | "paused" | "error";
  currentChannel: XtreamChannel | null;
  volume: number;
  bitrateKbps: number | null;
}
```

## 10. Build order (suggested milestones for Claude Code)

0. **Before writing any matching or URL logic:** hit the real APIs and inspect actual responses.
   - `get_live_categories` and `get_live_streams` for the configured group — confirm field names/shapes (`num`, `name`, `stream_icon`, `stream_id`) match what's assumed in this doc
   - `/nowplaying` from StellarTunerLog — pull real channel name strings and compare them side by side against the Xtream `name` values for the same channels, to see how close the naming actually is before tuning the fuzzy matcher
   - One live channel's stream URL with both `.ts` and `.m3u8` — confirm which resolves, and whether it's consistent across channels or needs per-channel/per-provider detection
   - Write down what's actually observed (a short markdown note or comment block is fine) before milestone 2 — the rest of the build should work from real samples, not from this doc's assumptions
1. Tauri scaffold + Mantine AppShell shell, no real data yet — confirm it launches on both target OSes
2. Settings screen + Xtream `get_live_categories`/`get_live_streams`, channel list rendering ordered by `num`
3. mpv sidecar + IPC bridge in Rust, wire up play/pause/volume from a hardcoded channel to prove the pipe works
4. Wire channel selection → mpv `loadfile`, confirm audio actually plays end to end
5. StellarTunerLog polling + `channelMatcher`, now-playing panel
6. Now-playing subtitle on channel list rows (reuse the same matched map)
7. Bitrate via `observe_property`/`packet-audio-bitrate` fallback
8. Credential storage via keyring, replace any plaintext secrets from earlier milestones
9. OS media integration (souvlaki) — push metadata/playback state on every channel/track change, wire Play/Pause/Next/Previous events back to the player
10. Packaging: `tauri build` for `.msi`/`.exe` and `.AppImage`/`.deb`

## 11. Open questions to verify against your real instance before/while building

- Whether the live backend serves `.ts` or `.m3u8` consistently across all channels, or whether it varies — decide if per-channel auto-detection (milestone 0) is actually needed or a single global setting is enough
- How closely Xtream `name` strings actually match StellarTunerLog's channel names in practice — pull a real sample from both and tune the normalization/fuzzy-match threshold against it
- Whether `audio-bitrate` populates live for this stream's container, or whether `packet-audio-bitrate` is the only reliable source in practice
- Whether the Windows/Linux "Next"/"Previous" media-key events (which souvlaki surfaces regardless of whether the app supports track skipping) should be mapped to anything — likely just ignored/no-op for a radio tuner, but decide explicitly rather than leaving them unhandled

## 12. VS Code tasks

Add `.vscode/tasks.json` so dev/build/lint are one command from the Command Palette (`Tasks: Run Task`) instead of remembering CLI incantations:

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "dev",
      "type": "shell",
      "command": "npm run tauri dev",
      "group": { "kind": "build", "isDefault": true },
      "problemMatcher": []
    },
    {
      "label": "build (current OS)",
      "type": "shell",
      "command": "npm run tauri build",
      "group": "build",
      "problemMatcher": []
    },
    {
      "label": "lint",
      "type": "shell",
      "command": "npm run lint",
      "group": "test",
      "problemMatcher": []
    },
    {
      "label": "typecheck",
      "type": "shell",
      "command": "npx tsc --noEmit",
      "group": "test",
      "problemMatcher": ["$tsc"]
    },
    {
      "label": "cargo check (src-tauri)",
      "type": "shell",
      "command": "cargo check",
      "options": { "cwd": "${workspaceFolder}/src-tauri" },
      "group": "test",
      "problemMatcher": ["$rustc"]
    }
  ]
}
```

`dev` is marked as the default build task (`Ctrl+Shift+B` / `Cmd+Shift+B` runs it directly). `build (current OS)` only produces installers for whichever OS you're on — cross-platform releases still go through the GitHub Actions workflow, not this task. Add `npm run lint`/`tsc` scripts to `package.json` if they don't already exist as part of the scaffold in milestone 1.

