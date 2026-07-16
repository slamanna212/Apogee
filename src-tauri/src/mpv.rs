use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// Per-process IPC path (rather than a single fixed name) so a stale/
/// abandoned socket from another instance can never collide with, or be
/// pre-created ahead of, this one - see `ipc_path`.
#[cfg(unix)]
fn ipc_path() -> &'static str {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| format!("/tmp/apogee-mpv-{}.sock", std::process::id()))
}
#[cfg(windows)]
fn ipc_path() -> &'static str {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| format!(r"\\.\pipe\apogee-mpv-{}", std::process::id()))
}

const STDERR_TAIL_LINES: usize = 40;

/// Caps applied to `read_bounded_line` so a malformed or hostile write to the
/// mpv IPC socket/stderr pipe (see `ipc_path`'s doc comment for why the
/// socket path alone isn't a complete guarantee against another local
/// process connecting) can't grow the read buffer unbounded by never sending
/// a newline.
const MAX_IPC_LINE_BYTES: usize = 1024 * 1024;
const MAX_STDERR_LINE_BYTES: usize = 64 * 1024;

/// Reads one `\n`-terminated line, capping accumulated bytes at `max_len`
/// unlike `AsyncBufReadExt::lines()`, which has no bound and will grow its
/// buffer indefinitely if a line is never terminated. Returns `Ok(None)` on
/// clean EOF with no partial line pending.
async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max_len: usize,
) -> std::io::Result<Option<String>> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if buf.is_empty() {
                Ok(None)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "stream ended mid-line",
                ))
            };
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            buf.extend_from_slice(&available[..=pos]);
            reader.consume(pos + 1);
            break;
        }
        if buf.len() + available.len() > max_len {
            let discard = available.len();
            reader.consume(discard);
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("line exceeded {max_len} bytes"),
            ));
        }
        buf.extend_from_slice(available);
        let n = available.len();
        reader.consume(n);
    }
    while matches!(buf.last(), Some(b'\n' | b'\r')) {
        buf.pop();
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

struct Inner {
    child: Child,
    writer: tokio::io::WriteHalf<Box<dyn AsyncReadWrite>>,
}

#[derive(Default)]
pub struct MpvState {
    inner: Arc<Mutex<Option<Inner>>>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
}

/// Prefers a bundled mpv (Windows, Linux AppImage) over PATH lookup (Linux
/// deb/rpm, which declare mpv as a package dependency instead, and macOS,
/// which has no bundled binary and relies on the user installing mpv via
/// Homebrew) - see docs/milestone-0-findings.md and the plan this
/// implements for why bundling isn't done on every platform.
// Scans $PATH by hand (rather than just returning the bare name and letting
// exec's own PATH search handle it) so callers can tell *before* spawning
// whether a real system binary is available, in order to prefer it over a
// bundled fallback.
#[cfg(target_os = "linux")]
fn find_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn resolve_mpv_path(app: &AppHandle) -> String {
    // .deb/.rpm installs declare `mpv` as a package dependency (see
    // tauri.conf.json's bundle.linux.deb/rpm.depends), so a real system mpv
    // is normally already on PATH there - prefer it over the bundled binary.
    // The bundled copy exists only as a fallback for the AppImage format
    // (which has no dependency mechanism of its own) or a missing/broken
    // system install; unconditionally preferring it would mean a stale or
    // corrupted bundled binary silently shadows a perfectly working system
    // mpv, which is exactly what happened during local dev testing.
    #[cfg(target_os = "linux")]
    if let Some(path) = find_on_path("mpv") {
        return path.to_string_lossy().into_owned();
    }

    if let Ok(resource_dir) = app.path().resource_dir() {
        let bundled: PathBuf = resource_dir.join(if cfg!(windows) {
            "binaries/mpv.exe"
        } else {
            "binaries/mpv"
        });
        if bundled.exists() {
            return bundled.to_string_lossy().into_owned();
        }
    }

    // Apps launched from Finder/Dock (as opposed to a Terminal) don't inherit
    // the interactive shell's PATH, so a bare "mpv" lookup often misses
    // Homebrew (or MacPorts) even when installed. Check known install
    // prefixes explicitly before falling back to plain PATH resolution
    // (which still covers any other manual install already on PATH).
    #[cfg(target_os = "macos")]
    for candidate in [
        "/opt/homebrew/bin/mpv",
        "/usr/local/bin/mpv",
        "/opt/local/bin/mpv",
    ] {
        if std::path::Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }

    "mpv".to_string()
}

async fn push_stderr_tail(tail: &Arc<Mutex<VecDeque<String>>>, line: String) {
    let mut guard = tail.lock().await;
    if guard.len() >= STDERR_TAIL_LINES {
        guard.pop_front();
    }
    guard.push_back(line);
}

async fn stderr_tail_string(tail: &Arc<Mutex<VecDeque<String>>>) -> String {
    tail.lock()
        .await
        .iter()
        .cloned()
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Masks the Xtream username/password segment of a stream URL
/// (`{baseUrl}/live/{user}/{pass}/{streamId}{extension}`) so debug logging of
/// commands/URLs can't leak credentials into the exported log file.
fn redact_credentials(url: &str) -> String {
    let Some((prefix, rest)) = url.split_once("/live/") else {
        return url.to_string();
    };
    let mut parts = rest.splitn(3, '/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(_user), Some(_pass), Some(tail)) => format!("{prefix}/live/***/***/{tail}"),
        _ => format!("{prefix}/live/***"),
    }
}

/// Renders an mpv IPC command for debug logging, redacting the URL argument
/// of a `loadfile` command via `redact_credentials`.
fn describe_command_for_log(cmd: &Value) -> String {
    let mut cmd = cmd.clone();
    if let Some(arr) = cmd.get_mut("command").and_then(|c| c.as_array_mut()) {
        if arr.first().and_then(|v| v.as_str()) == Some("loadfile") {
            if let Some(url) = arr.get_mut(1) {
                if let Some(s) = url.as_str() {
                    *url = Value::String(redact_credentials(s));
                }
            }
        }
    }
    cmd.to_string()
}

/// Kills the mpv child process, if any, without needing an async context.
/// Tauri's shutdown path (`RunEvent::Exit`) is synchronous and typically ends
/// in `std::process::exit`, which skips Drop impls - so `kill_on_drop` on the
/// child alone is not enough to guarantee mpv (and its open stream
/// connection) is torn down when the app quits.
///
/// This is a best-effort, fire-and-forget signal only (it doesn't wait for
/// mpv to actually exit) - it's a fast path for the normal-quit case. The
/// durable safety net for *any* kind of Apogee exit (crash, force-quit, the
/// Windows updater's `std::process::exit`, which also never runs this) is
/// `create_process_job`/`pre_exec` PDEATHSIG below, which the OS enforces
/// even if this function never runs at all.
pub fn kill_on_exit(state: &MpvState) {
    if let Ok(mut guard) = state.inner.try_lock() {
        if let Some(inner) = guard.as_mut() {
            let _ = inner.child.start_kill();
        }
    }
}

/// Kills mpv and blocks until it has actually exited (and released its file
/// handle), unlike `kill_on_exit`'s fire-and-forget signal. Used right before
/// the Windows updater launches the NSIS/MSI installer, which will fail to
/// overwrite `mpv.exe` if it's still running.
///
/// Deliberately synchronous (`try_lock`/`try_wait` + `thread::sleep`, no
/// `.await`) rather than an `async fn`: the updater plugin's `on_before_exit`
/// hook is a plain `Fn() + Send + Sync`, invoked from inside an already-running
/// async command handler on Tauri's tokio runtime - nesting a `block_on` in
/// that position would panic ("Cannot start a runtime from within a
/// runtime"). Blocking the current OS thread with a short poll loop instead
/// sidesteps that entirely.
#[cfg(windows)]
pub fn kill_blocking(state: &MpvState) {
    for _ in 0..50 {
        match state.inner.try_lock() {
            Ok(mut guard) => {
                let Some(inner) = guard.as_mut() else { return };
                let _ = inner.child.start_kill();
                for _ in 0..50 {
                    match inner.child.try_wait() {
                        Ok(Some(_)) => return,
                        _ => std::thread::sleep(std::time::Duration::from_millis(100)),
                    }
                }
                return;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
}

/// Puts the *current* (Apogee) process into a Windows Job Object configured
/// to kill every process in the job as soon as the job's last handle closes
/// - which happens automatically when this process terminates, by any means
/// (normal exit, crash, task-kill, or the updater's `std::process::exit`).
/// Child processes spawned afterwards (mpv) join the job automatically, so
/// this guarantees mpv can never outlive Apogee, without depending on Drop or
/// Tauri's event loop running at all. Must be called once at startup and the
/// returned `Job` kept alive for the process lifetime (e.g. via Tauri managed
/// state) - dropping it early would close the handle and kill mpv
/// prematurely.
///
/// Also sets `JOB_OBJECT_LIMIT_BREAKAWAY_OK` (non-silent breakaway). By
/// default *every* child process automatically joins this same job, which
/// used to also silently kill the Windows updater's installer the instant
/// Apogee exits post-handoff (before it could even show its progress
/// window). This flag doesn't change that default by itself - a child still
/// joins the job unless it explicitly passes `CREATE_BREAKAWAY_FROM_JOB` at
/// creation time, which `spawn_mpv` never does - it only makes that escape
/// hatch available for the one spawn that now deliberately uses it (the
/// updater's installer launch in `updater.rs`).
#[cfg(windows)]
pub fn create_process_job() -> Option<win32job::Job> {
    let job = match win32job::Job::create() {
        Ok(job) => job,
        Err(e) => {
            log::warn!("failed to create process job object: {e}");
            return None;
        }
    };

    let mut info = match job.query_extended_limit_info() {
        Ok(info) => info,
        Err(e) => {
            log::warn!("failed to query process job object limits: {e}");
            return None;
        }
    };
    info.limit_kill_on_job_close();
    info.limit_breakaway_ok();
    if let Err(e) = job.set_extended_limit_info(&info) {
        log::warn!("failed to configure process job object: {e}");
        return None;
    }

    if let Err(e) = job.assign_current_process() {
        log::warn!("failed to assign process to job object: {e}");
        return None;
    }

    Some(job)
}

#[cfg(unix)]
async fn connect_ipc(path: &str) -> std::io::Result<Box<dyn AsyncReadWrite>> {
    Ok(Box::new(tokio::net::UnixStream::connect(path).await?))
}

#[cfg(windows)]
async fn connect_ipc(path: &str) -> std::io::Result<Box<dyn AsyncReadWrite>> {
    Ok(Box::new(
        tokio::net::windows::named_pipe::ClientOptions::new().open(path)?,
    ))
}

fn spawn_mpv(mpv_path: &str, user_agent: &str) -> std::io::Result<Child> {
    let mut cmd = Command::new(mpv_path);
    cmd.args([
        "--no-video",
        "--idle=yes",
        &format!("--input-ipc-server={}", ipc_path()),
        &format!("--user-agent={user_agent}"),
    ])
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .kill_on_drop(true);

    // Job objects (Windows, see `create_process_job`) cover the general "any
    // kind of Apogee exit" case, but Linux has no equivalent - ask the kernel
    // to SIGTERM mpv itself if Apogee's process dies for any reason (crash,
    // force-quit, `kill -9`) so it's not left running/orphaned.
    //
    // macOS has no PR_SET_PDEATHSIG or Job Object equivalent, so this
    // abnormal-exit case (crash/force-quit only - normal quit and the
    // updater's relaunch are already covered by kill_on_exit/kill_blocking,
    // which run unconditionally on every platform) is left unhandled there.
    // Closing it fully would need an external watcher process polling the
    // parent PID via kqueue/EVFILT_PROC, which is disproportionate to the
    // risk (a briefly orphaned mpv after an already-abnormal crash, not data
    // loss) - accepted as a known limitation, not a TODO.
    #[cfg(target_os = "linux")]
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM as libc::c_ulong);
            Ok(())
        });
    }

    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);

        // mpv >= 0.38 registers its own Windows SMTC (System Media Transport
        // Controls) session by default, which competes with the app's own
        // souvlaki-based media session (see media_session.rs) and wins the OS
        // "now playing" overlay with the raw stream filename and no artwork.
        // Disable mpv's built-in media controls so only our session shows.
        //
        // This is gated to Windows only (rather than passed unconditionally)
        // because mpv treats an unrecognized option as a fatal startup error,
        // not a silent no-op (verified: "Error parsing option ... (option not
        // found)" -> "Exiting... (Fatal error)"). `--media-controls` didn't
        // exist before mpv v0.38.0, and this project doesn't pin a minimum
        // mpv version, so passing it unconditionally risks breaking playback
        // entirely for anyone (on any OS) running an older mpv build. The bug
        // this addresses is Windows-only anyway (SMTC doesn't exist on
        // Linux/macOS), so scoping the flag to Windows avoids that risk while
        // still fixing the reported issue.
        cmd.arg("--media-controls=no");
    }

    cmd.spawn()
}

async fn ensure_started(app: &AppHandle, state: &MpvState) -> Result<(), String> {
    let mut guard = state.inner.lock().await;
    if guard.is_some() {
        return Ok(());
    }

    #[cfg(unix)]
    let _ = std::fs::remove_file(ipc_path());

    state.stderr_tail.lock().await.clear();

    let mpv_path = resolve_mpv_path(app);
    let version = app.package_info().version.to_string();
    let os_label = match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    };
    let user_agent = format!("Apogee/{version} ({os_label})");
    let mut child = spawn_mpv(&mpv_path, &user_agent).map_err(|e| {
        let message = if e.kind() == std::io::ErrorKind::NotFound {
            "mpv not found - install it (macOS: `brew install mpv`; Linux: install the `mpv` package) and restart Apogee".to_string()
        } else {
            format!("failed to spawn mpv: {e}")
        };
        log::error!("{message}");
        message
    })?;

    if let Some(stderr) = child.stderr.take() {
        let tail = state.stderr_tail.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            loop {
                match read_bounded_line(&mut reader, MAX_STDERR_LINE_BYTES).await {
                    Ok(Some(line)) => {
                        log::warn!("mpv: {line}");
                        push_stderr_tail(&tail, line).await;
                    }
                    Ok(None) => break,
                    Err(e) => {
                        log::warn!("mpv stderr line stream ended abnormally: {e}");
                        break;
                    }
                }
            }
        });
    }

    let mut stream = None;
    for _ in 0..50 {
        if let Ok(Some(status)) = child.try_wait() {
            let tail = stderr_tail_string(&state.stderr_tail).await;
            let suffix = if tail.is_empty() {
                String::new()
            } else {
                format!(" - {tail}")
            };
            let message = format!("mpv exited immediately (status: {status}){suffix}");
            log::error!("{message}");
            return Err(message);
        }
        match connect_ipc(ipc_path()).await {
            Ok(s) => {
                // Restrict the socket to the owning user - by default it's
                // created with whatever mpv's umask produces, which could
                // let another local user on a shared machine connect and
                // drive mpv's full remote-control IPC surface.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Err(e) = std::fs::set_permissions(
                        ipc_path(),
                        std::fs::Permissions::from_mode(0o600),
                    ) {
                        log::warn!("failed to restrict mpv IPC socket permissions: {e}");
                    }
                }
                stream = Some(s);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
    let stream = match stream {
        Some(s) => s,
        None => {
            let _ = child.start_kill();
            let tail = stderr_tail_string(&state.stderr_tail).await;
            let suffix = if tail.is_empty() {
                String::new()
            } else {
                format!(" - mpv output: {tail}")
            };
            let message = format!("timed out connecting to mpv IPC{suffix}");
            log::error!("{message}");
            return Err(message);
        }
    };

    let (read_half, write_half) = tokio::io::split(stream);

    let app_clone = app.clone();
    let inner_for_reader = state.inner.clone();
    let stderr_tail_for_reader = state.stderr_tail.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(read_half);
        loop {
            let line = match read_bounded_line(&mut reader, MAX_IPC_LINE_BYTES).await {
                Ok(Some(line)) => line,
                Ok(None) => break,
                Err(e) => {
                    log::warn!("mpv IPC line stream ended abnormally: {e}");
                    break;
                }
            };
            match serde_json::from_str::<Value>(&line) {
                Ok(value) => {
                    // Every message mpv sends over IPC (property changes,
                    // pause/unpause, playback-restart, end-file, etc.) - the
                    // none-observed properties this doesn't cover are the
                    // main blind spot when chasing an intermittent playback
                    // issue, so log all of it at debug level rather than
                    // only the handful of events the frontend acts on.
                    log::debug!("mpv event: {value}");
                    let _ = app_clone.emit("mpv-event", value);
                }
                Err(e) => log::warn!("failed to parse mpv IPC line as JSON: {e} - line: {line}"),
            }
        }

        // The loop above only ends when mpv's process exited or the IPC
        // socket errored/closed - there is no separate task watching the
        // child's exit status once the initial connect handshake succeeds.
        // Without this, a mid-playback mpv crash left the frontend stuck on
        // status: 'playing' forever (no event, no log) and left the stale
        // `Inner` (dead child + closed writer) in place, so the next
        // mpv_load call would silently fail in send_command instead of
        // respawning mpv.
        let tail = stderr_tail_string(&stderr_tail_for_reader).await;
        let suffix = if tail.is_empty() {
            String::new()
        } else {
            format!(" - {tail}")
        };
        log::error!("mpv IPC connection closed unexpectedly{suffix}");
        *inner_for_reader.lock().await = None;
        let _ = app_clone.emit("mpv-event", json!({ "event": "apogee-ipc-closed" }));
    });

    *guard = Some(Inner {
        child,
        writer: write_half,
    });
    drop(guard);

    send_command(
        state,
        json!({ "command": ["observe_property", 1, "audio-bitrate"] }),
    )
    .await?;
    // core-idle (stalled waiting for data) and eof-reached surface rebuffering
    // /stall conditions mid-playback as debug-level property-change events -
    // the main signal this was missing for chasing intermittent Mac issues
    // that don't produce a hard error, just dead air.
    send_command(
        state,
        json!({ "command": ["observe_property", 2, "core-idle"] }),
    )
    .await?;
    send_command(
        state,
        json!({ "command": ["observe_property", 3, "eof-reached"] }),
    )
    .await?;

    Ok(())
}

async fn send_command(state: &MpvState, cmd: Value) -> Result<(), String> {
    // describe_command_for_log clones and walks the command JSON to redact
    // credentials before formatting it - skip that work entirely when debug
    // logging is off (the default), rather than relying on log::debug!'s own
    // level check, which only skips the *macro expansion's* formatting, not
    // evaluation of an argument expression computed ahead of the call.
    if log::log_enabled!(log::Level::Debug) {
        log::debug!("mpv command: {}", describe_command_for_log(&cmd));
    }
    let mut guard = state.inner.lock().await;
    let inner = guard
        .as_mut()
        .ok_or_else(|| "mpv not started".to_string())?;
    let mut payload = cmd.to_string();
    payload.push('\n');
    inner
        .writer
        .write_all(payload.as_bytes())
        .await
        .map_err(|e| {
            let message = format!("failed to write to mpv IPC: {e}");
            log::error!("{message}");
            message
        })
}

#[tauri::command]
pub async fn mpv_load(
    app: AppHandle,
    state: State<'_, MpvState>,
    url: String,
) -> Result<(), String> {
    // mpv's `loadfile` isn't restricted to http(s) media URLs - it can also
    // open local files or other protocol handlers mpv supports. The app
    // itself only ever constructs http(s) stream URLs (see buildStreamUrl in
    // src/lib/xtream.ts), so reject anything else here rather than
    // forwarding it to mpv unchecked.
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        let message = "mpv_load rejected: url must be http:// or https://".to_string();
        log::warn!("{message}");
        return Err(message);
    }

    log::debug!("mpv_load: {}", redact_credentials(&url));
    ensure_started(&app, &state).await?;
    send_command(&state, json!({ "command": ["loadfile", url, "replace"] })).await
}

#[tauri::command]
pub async fn mpv_stop(state: State<'_, MpvState>) -> Result<(), String> {
    send_command(&state, json!({ "command": ["stop"] })).await
}

#[tauri::command]
pub async fn mpv_set_volume(state: State<'_, MpvState>, volume: u8) -> Result<(), String> {
    send_command(
        &state,
        json!({ "command": ["set_property", "volume", volume] }),
    )
    .await
}

#[tauri::command]
pub async fn mpv_set_property(state: State<'_, MpvState>, name: String, value: Value) -> Result<(), String> {
    send_command(&state, json!({ "command": ["set_property", name, value] })).await
}

/// Fixed request_id so the frontend can correlate the async IPC reply on the
/// shared "mpv-event" stream without a full request/response tracking table.
pub const GET_PROPERTY_REQUEST_ID: i64 = 777;

#[tauri::command]
pub async fn mpv_get_property(state: State<'_, MpvState>, name: String) -> Result<(), String> {
    send_command(
        &state,
        json!({ "command": ["get_property", name], "request_id": GET_PROPERTY_REQUEST_ID }),
    )
    .await
}

/// Exposes mpv's recent stderr output (already tracked in `stderr_tail` for
/// spawn-failure error messages) so the frontend can attach diagnostic
/// context when a load stalls or fails after mpv started successfully.
#[tauri::command]
pub async fn mpv_get_stderr_tail(state: State<'_, MpvState>) -> Result<String, String> {
    Ok(stderr_tail_string(&state.stderr_tail).await)
}
