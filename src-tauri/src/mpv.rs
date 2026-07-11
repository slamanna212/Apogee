use serde_json::{json, Value};
use std::process::Stdio;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

#[cfg(unix)]
const IPC_PATH: &str = "/tmp/pulsar-mpv.sock";
#[cfg(windows)]
const IPC_PATH: &str = r"\\.\pipe\pulsar-mpv";

trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

struct Inner {
    #[allow(dead_code)]
    child: Child,
    writer: tokio::io::WriteHalf<Box<dyn AsyncReadWrite>>,
}

#[derive(Default)]
pub struct MpvState {
    inner: Arc<Mutex<Option<Inner>>>,
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

fn spawn_mpv() -> std::io::Result<Child> {
    Command::new("mpv")
        .args([
            "--no-video",
            "--idle=yes",
            &format!("--input-ipc-server={IPC_PATH}"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
}

async fn ensure_started(app: &AppHandle, state: &MpvState) -> Result<(), String> {
    let mut guard = state.inner.lock().await;
    if guard.is_some() {
        return Ok(());
    }

    #[cfg(unix)]
    let _ = std::fs::remove_file(IPC_PATH);

    let child = spawn_mpv().map_err(|e| format!("failed to spawn mpv: {e}"))?;

    let mut stream = None;
    for _ in 0..50 {
        match connect_ipc(IPC_PATH).await {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
    let stream = stream.ok_or_else(|| "timed out connecting to mpv IPC".to_string())?;

    let (read_half, write_half) = tokio::io::split(stream);

    let app_clone = app.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(read_half).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                let _ = app_clone.emit("mpv-event", value);
            }
        }
    });

    *guard = Some(Inner {
        child,
        writer: write_half,
    });
    drop(guard);

    send_command(state, json!({ "command": ["observe_property", 1, "audio-bitrate"] })).await?;

    Ok(())
}

async fn send_command(state: &MpvState, cmd: Value) -> Result<(), String> {
    let mut guard = state.inner.lock().await;
    let inner = guard.as_mut().ok_or_else(|| "mpv not started".to_string())?;
    let mut payload = cmd.to_string();
    payload.push('\n');
    inner
        .writer
        .write_all(payload.as_bytes())
        .await
        .map_err(|e| format!("failed to write to mpv IPC: {e}"))
}

#[tauri::command]
pub async fn mpv_load(app: AppHandle, state: State<'_, MpvState>, url: String) -> Result<(), String> {
    ensure_started(&app, &state).await?;
    send_command(&state, json!({ "command": ["loadfile", url, "replace"] })).await
}

#[tauri::command]
pub async fn mpv_set_pause(state: State<'_, MpvState>, paused: bool) -> Result<(), String> {
    send_command(&state, json!({ "command": ["set_property", "pause", paused] })).await
}

#[tauri::command]
pub async fn mpv_set_volume(state: State<'_, MpvState>, volume: u8) -> Result<(), String> {
    send_command(&state, json!({ "command": ["set_property", "volume", volume] })).await
}
