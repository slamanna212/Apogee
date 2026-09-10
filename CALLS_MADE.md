# Calls made without you

Decisions taken during the overnight M5 run where your judgment would normally have been
asked. Every one is reversible; the file the change lives in is named so you can find it.

Authorised standing decisions (from you, before the run):
- Remove the Windows job object **and** the updater's breakaway logic. You have a Windows
  machine and will test dev builds and updates before releasing to main.
- Do the full API migration and drop `tauri-plugin-http`.
- Commit nothing. Everything is left in the working tree for you to review and stage.

---

(entries appended below as they happen)

## M5 — MPV removal

**Kept the custom Windows installer path, removed only its job-object coupling.**
The plan says to remove `CREATE_BREAKAWAY_FROM_JOB` but to *preserve installer
launch-failure reporting*. Those are separable, so the breakaway flag and the MPV kill are
gone while the custom `install_windows` remains. It exists for a second, still-valid reason:
the updater plugin ignores the installer launch's return value and then exits the app
unconditionally, so a failed launch would leave you with no installer, no update, and no
error message. `src-tauri/src/updater.rs`.

**Linux package dependency swapped rather than dropped.** `deb.depends` and `rpm.depends`
were `["mpv"]`. CPAL links ALSA dynamically, so the runtime library is a genuine requirement
and is now declared as `libasound2` (deb) / `alsa-lib` (rpm). Declaring it is more correct
than relying on it being universally present, and it is a much smaller dependency than mpv.
`src-tauri/tauri.conf.json`.

**Deleted `scripts/fetch-mpv.mjs` and its two npm devDependencies** (`7zip-min`, `undici`).
Verified nothing else imports either. `package.json`.

**Dropped `rustfft` and `libc` from the app crate.** `rustfft` moved to `playback-core` with
the analyser; `libc` was only used by the Linux capture path. Neither has any remaining
reference. `src-tauri/Cargo.toml`.

**Rewrote the macOS entitlements comment.** The file stays intentionally empty, but its
explanation referenced Homebrew mpv discovery as the reason for not sandboxing. That reason
is gone; the file now records that no capture or microphone entitlement is needed either.
`src-tauri/entitlements.plist`.

**Fixed one pre-existing clippy warning** (`needless_borrow` in `lib.rs`) so the workspace is
warning-clean. Unrelated to the migration, trivially revertible.

**Kept historical comments as history.** Several comments still mention mpv where they explain
why something is the way it is (the Waveform component's note about af-metadata, the updater's
note about why a custom install path exists). The plan asks for historical findings to stay
clearly historical rather than be deleted.

## M5 — API migration and plugin-http removal

**Responses pass through as raw JSON.** `xtream.rs` and `stellar.rs` return
`serde_json::Value` rather than typed Rust structs. Defining Rust mirrors of every Xtream
and StellarTunerLog field would have been a second place for those shapes to drift, and the
existing TypeScript types and their tests already describe them. Rust owns the parts that
must not be got wrong (URL construction, credential escaping, error redaction); the shapes
stay where they were.

**Two `NetworkService` instances, one configuration.** Tauri manages one for command
handlers; `NetworkService::shared()` serves Last.fm and notification artwork, which are
reached from places with no `State` to thread through. Threading `State` into those would
have meant changing signatures well outside this migration's blast radius.

**Notification artwork kept its 3 MiB cap.** The shared service defaulted to 8 MiB. Rather
than silently tripling what gets cached, the service default is now 3 MiB, matching the
constant that used to live in `notifications.rs`. A test asserts it, so the move cannot
quietly loosen it later. `src-tauri/src/network.rs`.

**Last.fm's error path now reads the body of non-2xx responses.** Last.fm reports API errors
in the body *with* an error status, so the previous behaviour depended on reading it. The
shared reader rejects unsuccessful statuses, so a variant that returns the body anyway was
added rather than losing the API's actual error message.

**`buildStreamUrl` deleted from the frontend, with its tests.** Rust builds stream URLs now.
The three frontend tests covering escaping and trailing slashes have equivalents in
`src-tauri/src/playback/commands.rs`, which is why the frontend test count dropped by three.

**Repository identifier validated in `github_releases`.** It comes from a frontend constant,
but it is interpolated into a URL path, so it is checked for the `owner/name` shape with no
query, fragment, or traversal. Tested.

**The wildcard `http:default` capability was removed** along with the plugin. It allowed the
webview to fetch any http/https URL; nothing needs that now.

**Channel artwork was NOT moved behind a Rust cache.** The plan suggests one, but those URLs
are consumed by `<img>` tags, so the webview performs the request; this is framework-owned
traffic, which the plan explicitly allows provided it is documented. Moving it would mean a
new asset-protocol cache and touching every logo render path, which is a feature rather than
a migration step. The SiriusXM CDN https-to-http downgrade therefore stays in TypeScript,
because the webview is what performs the failing TLS handshake.

## Formatting

**`src-tauri/src/media_session.rs` carries a formatting-only diff.** No logic changed; it is
rustfmt reflowing four struct literals. Earlier in this work I reverted exactly this kind of
diff as noise, and the reasoning has changed: with `mpv.rs` and `waveform.rs` deleted, this
was the last non-conformant file in the crate, so leaving it meant `cargo fmt --check` (a
command the plan lists under verification) would keep failing on an otherwise clean tree.

If you would rather keep the diff minimal, `git checkout -- src-tauri/src/media_session.rs`
reverts it with no other effect.

## Retry budget regression (found during your testing, 2026-09-10)

**Symptom:** tuning a channel failed with HTTP 503 after about five seconds, but the same
channel played if you tried again a couple of minutes later.

**Cause, mine.** The old implementation retried four times, but each attempt could sit for up
to `CONNECT_TIMEOUT_MS` (20 s) waiting for a stream, so four attempts spanned over a minute.
I preserved the attempt count and the 1.5 s delay but not that wall-clock window. A 503
fails instantly, so all four attempts burned in five seconds:

```
14:36:28  tune
14:36:28, :30, :31, :33   four 503s
14:36:33  gave up
```

**Fix.** The connect phase is now bounded by **two** independent limits, and giving up
requires both to be spent: the four-attempt count, and a 90-second wall-clock budget
(`CONNECT_BUDGET_MS`). Delays back off exponentially from 1.5 s to a 8 s cap, so a
fast-failing endpoint is retried across the whole window without being hammered.

**Deliberately not tuned to your setup.** I started to measure how long your Dispatcharr
takes to spin a stream up and stopped: sizing the retry window to one backend would be wrong
for anyone connecting straight to a provider. The budget restores the window the app always
effectively had, and is backend-agnostic. The plan already records first-attempt failures
during "upstream channel spin-up" as a known transient condition for direct providers too.

Three tests cover it, including one asserting that when every failure is free the retries
still span the full budget, and one asserting the request count inside that window stays
modest so the fix does not become a hammering bug.

## Silent failure: server error messages were being discarded

**Symptom:** three channels failed with a bare "503 Service Unavailable" and no indication
of why. They never recovered, while other channels played fine.

**Actual cause, not ours.** Those channels had no upstream stream assigned in Dispatcharr,
which says so plainly in the response body:

```
{"error": "No streams assigned to channel", "waited": "0s"}
```

**Our bug was hiding it.** `NetworkError::Status` carried only a `StatusCode` and threw the
body away, so an actionable message became an opaque status. Fixed in three parts:

- `NetworkError::Status` now carries an optional `detail`, read from the error body with a
  512-byte ceiling and a 2-second timeout so a failure path cannot stall. JSON
  `error`/`message`/`detail` fields are unwrapped, since that is how these proxies report a
  reason; anything else falls back to a trimmed prefix. Always redacted, because a body can
  echo the request URL.
- `SourceError::Status { status, detail }` keeps the status code instead of stringifying it,
  and leads its message with the server's own reason.
- Retry classification now distinguishes them: **4xx is permanent** (the request itself is
  wrong, repeating it unchanged cannot succeed) while **5xx stays retryable** (the server
  could not serve it *now*, which is the documented upstream spin-up case).

I deliberately did **not** special-case the "No streams assigned" string. Matching a
particular proxy's wording would break for anyone connecting directly to a provider. A 503
with a body is still retried, but now the user can see why from the first attempt.

Three tests cover it, including one asserting a credential in an error body never survives
into a surfaced message.
