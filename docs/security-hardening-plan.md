# Security hardening plan

Execution plan for seven findings from the security review of Apogee at
`b267cb4`. Each item is self-contained: do them in order, verify each before
moving on.

Every claim marked **[verified]** was reproduced by running the code, not
inferred from reading it.

**Toolchain note.** The repo pins Node `>=24 <25` with `engine-strict=true`
in `.npmrc`. Use Node 24 or `npm ci` refuses to run. The Rust side needs the
bundled mpv present before `cargo check` will succeed (`tauri.linux.conf.json`
declares `binaries/mpv` as a bundle resource): run
`node scripts/fetch-mpv.mjs auto` first if `src-tauri/binaries/` is empty.

**Verification commands** (run after each fix, all currently pass):

```bash
npm run build && npm run lint && npm test
cd src-tauri && cargo check --all-targets && cargo clippy --all-targets && cargo test
```

---

## 1. Redact Xtream credentials from mpv stderr

**Severity: high. Confirmed leak of the user's IPTV password into a file they
are likely to attach to a bug report.**

### The problem

`redact_credentials` in `src-tauri/src/mpv.rs` already guards `mpv_load` and
`describe_command_for_log`, but nothing guards the stderr reader.

**[verified]** Running the bundled mpv 0.41 against an Xtream-shaped URL, at
mpv's *default* message level — precisely what `mpv.rs:433` captures:

```
Failed to open http://127.0.0.1:9/live/myuser/mypassw0rd/12345.ts.
```

That line then travels:

1. `log::warn!("mpv: {line}")` → the rotating log file. It is `warn!`, so the
   `Info` default level does not suppress it — verbose logging is irrelevant.
2. `push_stderr_tail` → `stderr_tail` (last 40 lines).
3. `mpv_get_stderr_tail` → the frontend.
4. `playerStore.handleFailedAttempt` → `"Failed to connect after 4 attempts - <tail>"`
   → rendered on screen **and** re-logged via `logError`.
5. `export_log_file` → bundled into whatever the user uploads to a GitHub issue.

This is not an edge case. `MAX_CONNECT_ATTEMPTS = 4` alternating `.ts`/`.m3u8`
means ordinary channel changes produce failed opens.

### The fix

Redact at the single point where a stderr line is read, so every downstream
consumer inherits it.

The existing `redact_credentials` is not sufficient for free text. It handles
only the first `/live/` occurrence, and its fallback arm
(`_ => format!("{prefix}/live/***")`) *truncates* everything after the marker —
fine for a bare URL, destructive for a log sentence.

Replace it with a scanner that masks the two path segments following **every**
`/live/` occurrence and preserves all surrounding text. A path segment ends at
`/`, whitespace, `"`, or `'`. Keep the name `redact_credentials` (or rename to
`redact_stream_credentials` and update the two existing call sites — there are
no existing tests on it, so there is no test churn either way).

Required behaviour, as a unit-test table:

| Input | Expected output |
|---|---|
| `http://h:8080/live/user/pass/123.ts` | `http://h:8080/live/***/***/123.ts` |
| `Failed to open http://h:8080/live/user/pass/123.ts.` | `Failed to open http://h:8080/live/***/***/123.ts.` |
| `[ytdl_hook] Starting subprocess: [yt-dlp, --, http://h/live/u/p/1.ts]` | user and pass masked, rest byte-identical |
| one line containing two such URLs | both masked |
| `http://h/live/user/pass` (no trailing stream id) | `http://h/live/***/***` |
| `some log line with no marker` | unchanged |
| `trailing marker /live/` | unchanged, must not panic |

Apply it at these call sites in `src-tauri/src/mpv.rs`:

- **`ensure_started`, stderr task (~line 433)** — the primary fix. Redact
  before both the `log::warn!` and the `push_stderr_tail`:
  ```rust
  Ok(Some(line)) => {
      let line = redact_credentials(&line);
      log::warn!("mpv: {line}");
      push_stderr_tail(&tail, line).await;
  }
  ```
- **IPC read loop, `log::debug!("mpv event: {value}")` (~line 535)** — redact
  the rendered string.
- **IPC read loop, the JSON parse-failure warning (~line 543)** — it logs the
  raw `line` verbatim.

`stderr_tail_string` needs no change: redacting at ingestion means the stored
tail is already clean, which also covers `mpv_get_stderr_tail` and the
user-visible error string.

### Verify

Add the table above as `#[cfg(test)]` cases in `mpv.rs`. Then, manually:
configure a deliberately wrong Xtream password, attempt playback, and confirm
the exported log and the on-screen error both show `***` and never the password.

---

## 2. Stop mpv handing the stream URL to yt-dlp

**Severity: high. Confirmed disclosure of the Xtream password to every local
user via process arguments.**

### The problem

mpv ships `ytdl_hook.lua` as a **built-in script, enabled by default**. It
intercepts every URL mpv opens and shells out to `yt-dlp`/`youtube-dl` to test
whether it can extract a stream — regardless of whether the URL is already a
direct media stream. Apogee never asked for this; it is simply mpv's default
and `spawn_mpv` does not turn it off.

**[verified]** From the same run as finding 1:

```
[ytdl_hook] Starting subprocess: [yt-dlp, --no-warnings, -J, --flat-playlist, ...,
            --, http://127.0.0.1:9/live/myuser/mypassw0rd/12345.ts]
```

The credential-bearing URL is passed on another process's **command line**.
`/proc/<pid>/cmdline` is world-readable on a stock Linux system, so any local
user can read the Xtream password while a channel loads. It also fires three
times per failed open (yt-dlp, yt-dlp_x86, youtube-dl).

Apogee plays exactly one kind of URL: direct Xtream MPEG-TS/HLS. The extractor
is pure downside — a subprocess spawn, a large third-party parsing surface, and
a credential disclosure, for zero functionality.

### The fix

In `spawn_mpv` (`src-tauri/src/mpv.rs`), add `--no-ytdl` to the unconditional
argument list:

```rust
cmd.args([
    "--no-video",
    "--no-ytdl",
    "--idle=yes",
    &format!("--input-ipc-server={}", ipc_path()),
    &format!("--user-agent={user_agent}"),
])
```

Unconditional is correct here. Unlike `--media-controls` (mpv 0.38+, which is
why that one is Windows-gated with a comment explaining the fatal-unknown-option
risk), `--ytdl` has existed for roughly a decade and is present in every mpv
version Apogee could plausibly encounter. A command-line flag also overrides any
`ytdl=yes` in a user's `mpv.conf`.

Add a comment recording *why*, so nobody re-enables it: the hook leaks the
credential-bearing URL into subprocess argv and adds an extractor Apogee has no
use for.

### Verify

Run a channel with a bad password and confirm no `[ytdl_hook]` lines appear in
the exported log.

---

## 3. Allowlist the mpv property commands

**Severity: high (privilege boundary). Confirmed arbitrary file write.**

### The problem

`mpv_set_property(name, value)` and `mpv_get_property(name)` forward any
property name straight to mpv's IPC with no validation. mpv's property surface
includes options that write files at caller-chosen paths.

**[verified]** Against the bundled mpv over its IPC socket:

```
set_property log-file      <path>  → file created (8 KB)
set_property stream-record <path>  → file created (93 KB of stream bytes)
```

So any JavaScript executing in the webview gets an arbitrary-file-write
primitive — enough to drop a file into a shell profile or a startup directory.

Scope honestly: this requires webview JS execution first, and **no XSS vector
was found** (no `dangerouslySetInnerHTML`, no `eval`, no `innerHTML` anywhere;
React 19 blocks `javascript:` hrefs — **[verified]** by rendering one). It is a
defense-in-depth fix. `set_property scripts` was also tested hoping for code
execution: mpv accepts it but does not retro-load, so **there is no RCE path
here**, file-write only.

The fix is cheap because the frontend's real requirement is tiny.

### The fix

In `src-tauri/src/mpv.rs`, add two allowlists mirroring `secrets.rs`'s existing
`ALLOWED_FRONTEND_KEYS` pattern (same idea, same shape — follow it):

```rust
/// mpv properties the frontend is allowed to write. mpv's property surface
/// includes file-writing options (`log-file`, `stream-record`, ...), so an
/// unrestricted passthrough is an arbitrary-file-write primitive for any JS
/// in the webview. The frontend only ever needs these two.
const ALLOWED_SET_PROPERTIES: &[&str] = &["audio-device", "mute"];

/// Likewise for reads.
const ALLOWED_GET_PROPERTIES: &[&str] = &["packet-audio-bitrate"];
```

Reject anything else with an `Err` and a `log::warn!`, matching
`check_allowed_key`'s error style in `secrets.rs`.

These three names are the complete current usage — confirmed by grep:

| Name | Direction | Call sites |
|---|---|---|
| `audio-device` | set | `playerStore.ts:152`, `Settings.tsx:203`, `Settings.tsx:210` |
| `mute` | set | `mpvClient.ts:39` (`setMute`) |
| `packet-audio-bitrate` | get | `playerStore.ts:175` |

Do **not** widen this list "just in case". `mpv_set_volume`,
`mpv_set_equalizer` and `mpv_list_audio_devices` have their own typed commands
and go through `send_command` directly — they are unaffected, and the
`audio-device-list` read inside `mpv_list_audio_devices` does not route through
`mpv_get_property`, so it needs no allowlist entry.

### Verify

Unit tests: each allowed name passes the check; `log-file`, `stream-record` and
`scripts` are rejected. Manually confirm the audio-device picker in Settings and
the bitrate readout both still work.

---

## 4. Move the mpv IPC socket out of shared `/tmp`

**Severity: medium. Local-attacker race on multi-user machines.**

### The problem

`ipc_path()` returns `/tmp/apogee-mpv-<pid>.sock`. `ensure_started` removes any
existing file at that path, spawns mpv, then polls to connect.

On Linux, `/tmp` is world-writable. The path is fully predictable (the PID is
readable from `/proc`). Another local user can win the window between the
`remove_file` and mpv's `bind`, create and listen on that socket themselves, and
Apogee will connect to **their** socket. They then receive the `loadfile`
command — which carries the full stream URL including the Xtream username and
password — and can inject arbitrary `mpv-event` messages into the frontend's
state machine.

The existing `set_permissions(0o600)` does not help: it runs *after* connect, so
in the attack case it simply chmods the attacker's socket. The code comment
already acknowledges the path alone is not a complete guarantee; this closes it.

### The fix

Put the socket inside a directory only the owning user can traverse, so the
socket's own name stops mattering.

**Unix** (`#[cfg(unix)]` branch of `ipc_path`):

1. Pick a base directory, first match wins:
   - `$XDG_RUNTIME_DIR` if set and non-empty (Linux; already per-user, mode 0700),
   - else `std::env::temp_dir()` (on macOS this is the per-user
     `/var/folders/.../T/`, which is already private; on Linux it is `/tmp`,
     which is why the 0700 subdirectory below is required rather than optional).
2. Create `<base>/apogee-<uid>` with `std::fs::DirBuilder::new().mode(0o700).recursive(false).create(...)`.
   Setting the mode on the builder is required — creating then chmod-ing
   reintroduces the same race this fix exists to remove.
3. If creation returns `AlreadyExists`, `lstat` it and **require** that it is a
   real directory (not a symlink), owned by the current uid, with mode `0o700`.
   If any check fails, refuse to use it and return a hard error rather than
   silently downgrading.
4. Socket path becomes `<base>/apogee-<uid>/mpv-<pid>.sock`.

Keep the existing `remove_file` of a stale socket and the post-connect
`set_permissions(0o600)` — both are still correct, just no longer load-bearing.

**Windows** (`#[cfg(windows)]` branch): named pipes have no directory to hide
in, so make the name unguessable instead. Replace the PID with 128 bits of
randomness rendered as hex: `\\.\pipe\apogee-mpv-<32-hex>`. `getrandom` is
already in `Cargo.lock` transitively (0.2/0.3/0.4 are all present via the TLS
stack); add `getrandom = "=0.3.4"` as an explicit dependency to match the repo's
exact-pin convention in `Cargo.toml`. Using randomness on Unix too is harmless
and keeps the two branches symmetric, but the 0700 directory is what actually
closes the hole there.

`ipc_path()` currently returns `&'static str` from a `OnceLock<String>`. Since
directory creation can now fail, either keep that signature and have
`ensure_started` do the fallible directory setup before first use, or change it
to return `Result<&'static str, String>`. Either is fine; pick one and keep it
consistent across both `cfg` branches and the three existing call sites
(`ensure_started`'s `remove_file`, `spawn_mpv`'s argument, `connect_ipc`).

### Verify

Launch the app, play a channel, and confirm the socket lives at the new path
with the parent directory at `0700`. Confirm two concurrent instances still work
(distinct PIDs / distinct random names). Confirm cleanup on exit is unchanged.

---

## 5. Block webview navigation and fix external links

**Severity: medium. Also repairs a live functional bug.**

### The problem

Two related issues, and the review's initial framing of them was wrong in a way
that changes the work:

**5a — no navigation guard.** Nothing in `lib.rs` restricts where the webview
may navigate. The opener plugin injects a click interceptor that catches
`<a target="_blank">` and routes it to the system browser, so those links do
*not* navigate the app. But it only matches `_blank` (or ctrl/shift-click).
Plain links are untouched — and `react-markdown` in `UpdateModal` renders GitHub
release-note links as plain `<a href>` with no target. Clicking one replaces the
running app with a remote page.

**5b — external links are currently broken.** The opener interceptor invokes
`plugin:opener|open_url`, which is capability-scoped. `capabilities/default.json`
allows only `https://www.last.fm/api/auth/**`. So clicking a social link in
`ChannelModal` (Twitter/Facebook/mailto/tel, sourced from StellarTunerLog
metadata) calls `open_url`, gets denied by scope, and — because the interceptor
already called `preventDefault()` — **nothing happens at all**. This is a
user-visible bug today, not just a hardening concern.

### The fix

**5a. Navigation guard.** `tauri::Builder` has no `on_navigation` hook —
verified against `tauri-2.11.5`; it exists on `WebviewWindowBuilder` (unusable
here, since the window is declared in `tauri.conf.json`) and on
`tauri::plugin::Builder`. Register a small local plugin in `lib.rs`:

```rust
fn navigation_guard<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("apogee-navigation-guard")
        .on_navigation(|_webview, url| {
            let allowed = is_internal(url);
            if !allowed {
                log::warn!("blocked webview navigation to {}", url.scheme());
            }
            allowed
        })
        .build()
}
```

`is_internal` must allow every scheme/host Tauri itself uses, or the app will
not start:

- scheme `tauri:`, `asset:`, `ipc:`
- host `tauri.localhost`, `ipc.localhost`, `asset.localhost` (Windows serves the
  app over `http://` on these)
- `about:blank`
- in dev only (`cfg!(dev)`), host `localhost` / `127.0.0.1` for the Vite dev
  server on port 1420

**Log the decision before enforcing it.** Ship the guard in log-only mode first
(always return `true`, just log what *would* have been blocked), run the app on
Windows, macOS and Linux, and confirm nothing legitimate is logged. Then flip to
enforcing. Getting this list wrong produces a blank window with no obvious cause,
so the extra round trip is worth it.

**5b. A Rust-side external-open command.** Add to `src-tauri/src/lib.rs` (or a
small new module):

```rust
#[tauri::command]
pub fn open_external_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let parsed = url::Url::parse(&url).map_err(|e| e.to_string())?;
    if !matches!(parsed.scheme(), "https" | "mailto" | "tel") {
        return Err("only https, mailto and tel links can be opened".into());
    }
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_url(parsed.as_str(), None::<&str>)
        .map_err(|e| e.to_string())
}
```

The plugin's Rust API bypasses JS capability scope, so the `opener` capability
in `capabilities/default.json` stays narrow — do **not** widen it to
`https://**`. Note `http` is deliberately excluded: these are social/contact
links, and there is no reason to open a cleartext page in a browser.

Register it in the `invoke_handler!` list.

**5c. Route the two link sites through it.**

- `src/components/ChannelModal.tsx:209-227` — the socials `map`. Drop `href` and
  `target`, render a `<button>` (or keep the `<a>` with an `onClick` that calls
  `preventDefault`), and invoke `open_external_url`. Dropping `href` also stops
  the opener interceptor from double-firing. This is what makes the social links
  work again.
- `src/components/UpdateModal.tsx:74` — pass a `components` override to
  `react-markdown` so anchors render through the same path:
  ```tsx
  <Markdown components={{ a: ExternalLink }}>{entry.body}</Markdown>
  ```
  Put `ExternalLink` in a shared module (e.g. `src/lib/externalLink.tsx`) so both
  sites use one implementation.

### Verify

- Social links in the channel modal open in the system browser (they currently
  do nothing — confirm the before/after).
- A link inside release notes in the update modal opens in the browser and does
  **not** replace the app window.
- The app still starts and renders on all three platforms with the guard
  enforcing.

---

## 6. CI supply-chain hardening

**Severity: medium.**

### The problem

Three distinct issues in `.github/workflows/`:

1. **Actions are on mutable tags.** `actions/checkout@v7`,
   `actions/setup-node@v7`, `dtolnay/rust-toolchain@stable`,
   `Swatinem/rust-cache@v2`, `tauri-apps/tauri-action@v1`. Tags can be moved. In
   `release.yml` and `dev-prerelease.yml` these run in the same job that holds
   `TAURI_SIGNING_PRIVATE_KEY`, `STELLAR_API_KEY` and `LASTFM_SHARED_SECRET`.
2. **Third-party code runs on the signing runner pool.** `pr-ci.yml` routes both
   `slamanna212` *and* `dependabot[bot]` to the self-hosted `rapture-apogee`
   runner, then runs `npm ci` — which executes arbitrary lifecycle scripts from
   whatever dependency version the bot is proposing.
3. **No dependency-audit gate**, and `actions/checkout` leaves the job token in
   `.git/config` while that third-party code runs.

### The fix

**6a. Pin every action to a full commit SHA**, with the version as a trailing
comment, across all four workflow files:

```yaml
- uses: actions/checkout@<40-char-sha> # v7.x.y
```

Resolve each SHA from the upstream repo at the tag currently in use. Dependabot's
existing `github-actions` ecosystem entry keeps pinned SHAs updated, so this
costs nothing ongoing.

**6b. Keep Dependabot off the self-hosted runner.** In `pr-ci.yml`, both jobs
use:

```yaml
runs-on: ${{ contains(fromJSON('["slamanna212", "dependabot[bot]"]'), github.actor) && 'rapture-apogee' || 'ubuntu-latest' }}
```

Drop `dependabot[bot]` from that list so bot PRs land on `ubuntu-latest`:

```yaml
runs-on: ${{ github.actor == 'slamanna212' && 'rapture-apogee' || 'ubuntu-latest' }}
```

This is the highest-value change in this section and the lowest-risk. Prefer it
over `npm ci --ignore-scripts`, which risks breaking `oxlint`/`vite`'s
platform-binary resolution; if you also want `--ignore-scripts`, add it
separately and confirm lint and typecheck still pass before keeping it.

**6c. `persist-credentials: false`** on every `actions/checkout` step in
`pr-ci.yml` and the `build` jobs of `release.yml` / `dev-prerelease.yml`. Do
**not** add it to jobs that subsequently run `gh release ...` against the
checkout, and note the release jobs use `GH_TOKEN` from `env` rather than the
on-disk credential, so this is safe there — verify per job rather than applying
blindly.

**6d. Add a `security-audit` workflow** (`.github/workflows/security-audit.yml`):

- triggers: `pull_request`, plus `schedule` weekly, plus `workflow_dispatch`
- `permissions: contents: read`
- `runs-on: ubuntu-latest`
- steps: `npm ci` then `npm audit --audit-level=high`; install `cargo-audit`
  (`taiki-e/install-action` or `cargo install cargo-audit --locked`) then
  `cargo audit` in `src-tauri`.

**[verified]** plain `cargo audit` exits `0` on warnings-only and non-zero only
on real vulnerabilities, so it works as a gate with no ignore-list maintenance.
The repo's current state is 0 vulnerabilities with 17 warnings (all transitive
GTK3/glib/unic from Tauri's Linux stack), so this goes green on day one and only
fires on something new. Do **not** add `--deny warnings`; it would fail
immediately on unfixable upstream noise and get switched off.

### Verify

Open a scratch PR and confirm: PR CI runs on `ubuntu-latest` for a bot-authored
PR, the audit workflow passes, and a release dry run still builds.

---

## 7. Constrain notification-artwork fetches

**Severity: medium. Blind SSRF.**

### The problem

`send_os_notification(artwork_url)` passes the URL to `cache_artwork` in
`src-tauri/src/notifications.rs`, which fetches it with `reqwest`. The URL is
not the user's: `alertsStore.scan` sets it from
`station.artwork_url || channel?.stream_icon` — i.e. from StellarTunerLog or
from the Xtream provider.

`cache_artwork` validates the scheme (`http`/`https`) and caps the body at 3 MB,
but:

- it does not restrict the destination, so `http://127.0.0.1:<port>/...` or
  `http://169.254.169.254/...` are reachable;
- **redirects are followed by default and only the initial URL is checked** —
  this is the larger hole. The code's own use of `response.url()` for extension
  detection shows redirects do occur in practice.

Bodies are never returned to the caller, so this is blind SSRF — it can probe
and trigger, not exfiltrate.

### The fix

Do **not** simply require `https`. `downgradeSiriusCdnUrl` in
`src/lib/stellarTunerLog.ts` deliberately rewrites
`pri.art.prod.streaming.siriusxm.com` to `http://` because that host's TLS
certificate does not cover its own name. Requiring https would break channel
artwork.

Instead, block private destinations and re-check on every redirect hop.

1. Add a host classifier:
   ```rust
   /// Rejects hosts that should never be reachable from a remote-supplied
   /// artwork URL: loopback, private, link-local, unique-local and
   /// unspecified addresses, plus `localhost`.
   fn is_disallowed_artwork_host(url: &reqwest::Url) -> bool
   ```
   Handle both `Host::Ipv4`/`Host::Ipv6` literals (use `Ipv4Addr::is_loopback`,
   `is_private`, `is_link_local`, `is_unspecified`, `is_broadcast`; and
   `Ipv6Addr::is_loopback`, `is_unspecified`, plus explicit checks for `fc00::/7`
   unique-local and `fe80::/10` link-local) and `Host::Domain`, rejecting
   `localhost` and any `*.localhost`.
2. Apply it to the initial parsed URL, alongside the existing scheme check.
3. Give `ARTWORK_HTTP` a custom redirect policy instead of the default: use
   `reqwest::redirect::Policy::custom(...)` to reject any hop whose URL fails
   the same check, and cap the hop count (3 is plenty for a CDN).

A DNS name resolving to a private address still slips through — closing that
needs a custom resolver and is disproportionate here, since the attacker gets no
response body back. Note the limitation in a comment rather than pretending it
is fully solved.

### Verify

Unit tests for the classifier (a table of allowed and rejected hosts —
`127.0.0.1`, `10.0.0.1`, `192.168.1.1`, `169.254.169.254`, `::1`, `fd00::1`,
`localhost` rejected; a normal public host and the SiriusXM CDN host allowed).
Then confirm real channel artwork still appears in OS notifications — this is
the regression to watch for.

---

## Suggested order

1. **2** (`--no-ytdl`) — one line, removes a whole subprocess and its leak.
2. **1** (redaction) — self-contained, high value, unit-testable.
3. **3** (property allowlist) — self-contained, unit-testable.
4. **7** (SSRF) — self-contained, unit-testable.
5. **6** (CI) — touches no application code.
6. **4** (socket path) — needs manual multi-platform checking.
7. **5** (navigation guard) — riskiest; do it last, in log-only mode first.

Run the full verification command block after each item. Items 1–3 all touch
`mpv.rs`; doing them consecutively avoids repeated rebuild cycles.

---

## Explicitly out of scope

These were identified in the review and **deliberately excluded** from this
plan. They are recorded here so their absence is a decision, not an oversight.

| Item | Why deferred |
|---|---|
| `buildStreamUrl` does not `encodeURIComponent` credentials | Low: a password containing `/`, `?` or `#` silently alters the request path. |
| Xtream `baseUrl` scheme unvalidated; no cleartext-`http` warning | Low: UX/hygiene, not a new exposure. |
| CSP `connect-src` wider than needed (all traffic goes through the Rust http plugin, not webview fetch, so it could drop to `'self' ipc: http://ipc.localhost`) | Low, easy, but needs its own verification pass. |
| `export_log_file` can overwrite any writable `.log`/`.txt` | Low: already validated; moving the save dialog into Rust would remove the JS-supplied path entirely. |
| `dark_bg_color` from the remote API into an inline `style`; MD5 artwork cache keys | Very low. |
| `temp.txt` junk file at repo root | Housekeeping. |
| Baked-in `STELLAR_API_KEY` / `LASTFM_SHARED_SECRET` (recoverable with `strings` from any shipped build) | **Not fixable in code** — inherent to a client-side app. The mitigation is operational: treat both as public, rate-limit server-side, keep a rotation plan. Worth documenting in the README rather than "fixing". |
| `http:default` capability allows `http://*:*/*` and `https://*:*/*` | Unavoidable given user-configured Xtream base URLs. Could be narrowed by moving the fixed-host calls (`api.github.com`, `api.stellartunerlog.com`) into Rust. |
| mpv loads the user's `~/.config/mpv/mpv.conf` and auto-loads their `scripts/` (no `--no-config`) | Open question, raised and not resolved. The EQ code deliberately preserves non-Apogee filters, which suggests config loading is intentional for power users. It does mean a user's own mpv scripts can observe the credential-bearing stream URL. Decide separately. |
| 9 `cargo clippy` style warnings (doc formatting; 2 `needless_borrow` in `lib.rs`) | Not security. |

## Dependency posture at time of review

No action needed; recorded as a baseline.

- `npm audit` (prod and dev): **0 vulnerabilities**
- `cargo audit`: **0 vulnerabilities**, 17 warnings — 16 unmaintained + 1 unsound,
  all transitive GTK3/glib/unic from Tauri's Linux stack, nothing actionable
- `scripts/fetch-mpv.mjs` verifies a pinned SHA-256 before use — good, leave as is
- Frontend build, lint and 125 tests pass; Rust check, clippy and 8 tests pass
