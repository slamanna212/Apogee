//! Tauri adapter: typed commands and event translation.
//!
//! The engine and controller know nothing about Tauri. This module is the only place that
//! touches `AppHandle`, converting domain events into the `player-snapshot` event the
//! frontend consumes.
//!
//! Commands are typed rather than arbitrary property strings, so the frontend can no longer
//! reach into player internals the way it did with MPV's property protocol.

use std::sync::{Arc, Mutex};

use apogee_playback_core::session::{Controller, ErrorClass, Generation, Next, Snapshot};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use super::device::{list_output_devices, migrate_mpv_device, DeviceDescriptor, DeviceRequest};
use super::engine::{start_session, AudioSettings, EngineEvent, EventSink, Session};
use crate::network::NetworkService;

/// Enough for Rust to build both URL candidates itself. Credentials live here and in the
/// keyring only; they never appear in a snapshot or an event.
///
/// Field names are camelCase on the wire to match the TypeScript caller. Without the
/// rename, Tauri rejects the whole call with a "missing field" error before any of this
/// module runs, so the shape is pinned by a test below.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StationRequest {
    pub base_url: String,
    pub username: String,
    pub password: String,
    pub stream_id: i64,
    /// Shown in diagnostics and used as the snapshot's station identity.
    pub station_id: String,
}

impl StationRequest {
    /// Builds the stream URL for one attempt, preserving the existing provider path shape
    /// and the alternating extension behaviour.
    fn url_for(&self, extension: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!(
            "{base}/live/{}/{}/{}{extension}",
            urlencoding(&self.username),
            urlencoding(&self.password),
            self.stream_id
        )
    }
}

/// Percent-encodes a path segment. Provider credentials can contain characters that would
/// otherwise change the URL's structure.
fn urlencoding(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

struct Inner {
    controller: Controller,
    session: Option<Session>,
    settings: AudioSettings,
    device: DeviceRequest,
    request: Option<StationRequest>,
    attempt_serial: u64,
    active_attempt: Option<u64>,
}

impl Inner {
    fn begin_attempt(&mut self) -> u64 {
        self.attempt_serial = self.attempt_serial.wrapping_add(1);
        self.active_attempt = Some(self.attempt_serial);
        self.attempt_serial
    }

    fn accepts_attempt(&self, generation: Generation, attempt: u64) -> bool {
        self.controller.accepts(generation) && self.active_attempt == Some(attempt)
    }
}

/// Tauri-managed playback state.
pub struct PlayerState {
    inner: Mutex<Inner>,
    network: NetworkService,
}

impl PlayerState {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            inner: Mutex::new(Inner {
                controller: Controller::new(),
                session: None,
                settings: AudioSettings::default(),
                device: DeviceRequest::SystemDefault,
                request: None,
                attempt_serial: 0,
                active_attempt: None,
            }),
            network: NetworkService::new().map_err(|e| e.to_string())?,
        })
    }
}

/// Translates engine events into controller state and a Tauri event.
struct TauriSink {
    app: AppHandle,
    attempt: u64,
}

impl TauriSink {
    fn publish(&self, snapshot: Snapshot) {
        // A failed emit means the window is gone; nothing to recover.
        let _ = self.app.emit("player-snapshot", snapshot);
    }
}

impl EventSink for TauriSink {
    fn emit(&self, event: EngineEvent) {
        let state = self.app.state::<PlayerState>();
        let mut retry: Option<(Generation, String, u64)> = None;

        let snapshot = {
            let mut inner = state.inner.lock().expect("player state poisoned");
            if inner.active_attempt != Some(self.attempt)
                || event_generation(&event)
                    .is_some_and(|generation| !inner.controller.accepts(generation))
            {
                return;
            }
            match event {
                EngineEvent::Buffering { generation, reason } => {
                    inner.controller.on_buffering(generation, reason);
                }
                EngineEvent::Playing { generation } => {
                    inner.controller.on_audible(generation, now_ms());
                }
                EngineEvent::Format {
                    generation,
                    sample_rate,
                    ..
                } => {
                    inner.controller.set_format(generation, sample_rate);
                }
                EngineEvent::Bitrate { generation, kbps } => {
                    inner.controller.set_bitrate(generation, kbps);
                }
                EngineEvent::Spectrum { levels } => {
                    // Emitted directly rather than through the snapshot: it changes far too
                    // often to belong in playback state, and dropping one is harmless.
                    let _ = self.app.emit("waveform-levels", levels);
                    return;
                }
                EngineEvent::Underrun { .. } => {
                    // Observability only; the ring already emitted silence. Not a state change.
                }
                EngineEvent::Failed {
                    generation,
                    class,
                    message,
                } => {
                    let next = inner
                        .controller
                        .on_error(generation, class, message, now_ms());
                    if let (
                        Some(Next::Retry {
                            extension,
                            delay_ms,
                            ..
                        }),
                        Some(request),
                    ) = (next, inner.request.clone())
                    {
                        // The controller decides the delay: it backs off so a
                        // fast-failing endpoint is not hammered across the connect budget.
                        retry = Some((generation, request.url_for(extension), delay_ms));
                    }
                }
            }
            inner.controller.snapshot()
        };
        self.publish(snapshot);

        if let Some((generation, url, delay_ms)) = retry {
            schedule_retry(self.app.clone(), generation, self.attempt, url, delay_ms);
        }
    }
}

fn event_generation(event: &EngineEvent) -> Option<Generation> {
    match event {
        EngineEvent::Buffering { generation, .. }
        | EngineEvent::Playing { generation }
        | EngineEvent::Format { generation, .. }
        | EngineEvent::Bitrate { generation, .. }
        | EngineEvent::Underrun { generation, .. }
        | EngineEvent::Failed { generation, .. } => Some(*generation),
        // Spectrum samples are still scoped by the sink's attempt. They do not carry a
        // generation because the frontend payload has no playback-state semantics.
        EngineEvent::Spectrum { .. } => None,
    }
}

/// Milliseconds since an arbitrary, process-local, monotonic epoch (first call to this
/// function). `Controller` only ever compares two of these against each other, so the epoch
/// itself is meaningless - what matters is that this can never jump or go backwards the way
/// `SystemTime::now()` can (NTP step, user changing the clock, DST-adjacent bugs on some
/// platforms). Retry/health accounting must not be fooled by a system clock change.
fn now_ms() -> u64 {
    static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let epoch = EPOCH.get_or_init(std::time::Instant::now);
    epoch.elapsed().as_millis() as u64
}

/// Restarts a failed attempt after the retry delay, if the session is still current.
fn schedule_retry(
    app: AppHandle,
    generation: Generation,
    failed_attempt: u64,
    url: String,
    delay_ms: u64,
) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

        let state = app.state::<PlayerState>();

        // Take the failed session out and tear it down BEFORE reconnecting. The provider
        // allows only a small number of concurrent connections per account, so opening the
        // retry while the previous attempt is still holding one earns a 503 rather than a
        // stream. Dropping happens outside the lock: teardown joins the decode thread, and
        // that thread's event sink takes this same mutex.
        let (previous, settings, device, network, attempt) = {
            let mut inner = state.inner.lock().expect("player state poisoned");
            if !inner.accepts_attempt(generation, failed_attempt) {
                return; // The user moved on; abandon this attempt.
            }
            let attempt = inner.begin_attempt();
            (
                inner.session.take(),
                inner.settings.clone(),
                inner.device.clone(),
                state.network.clone(),
                attempt,
            )
        };
        drop(previous);

        let sink = Arc::new(TauriSink {
            app: app.clone(),
            attempt,
        }) as Arc<dyn EventSink>;
        match start_session(generation, url, &device, settings, network, sink) {
            Ok(session) => {
                let mut inner = state.inner.lock().expect("player state poisoned");
                if inner.accepts_attempt(generation, attempt) {
                    inner.controller.set_device(session.device_name());
                    inner.session = Some(session);
                } // else: `session` drops here, tearing straight back down.
            }
            Err(e) => {
                // Previously just logged and abandoned: the controller was left in
                // `Recovering` with no session, no scheduled retry, and no way out short of
                // the user manually stopping and replaying. Route it through the same
                // retry/give-up policy as any other failure instead.
                let snapshot = handle_startup_failure(&app, &state, generation, attempt, e);
                let _ = app.emit("player-snapshot", snapshot);
            }
        }
    });
}

/// The pure decision behind [`handle_startup_failure`]: given a failed [`start_session`]
/// call for `generation`, updates the controller and reports the resulting snapshot plus,
/// when another attempt is due, the URL and delay for it. Kept free of `AppHandle`/`State`
/// so this - the actual "retry or give up" policy - can be tested directly against a plain
/// [`Inner`], without a running Tauri app.
fn route_startup_failure(
    inner: &mut Inner,
    generation: Generation,
    message: String,
) -> (Snapshot, Option<(String, u64)>) {
    let next = inner
        .controller
        .on_error(generation, ErrorClass::Transient, message, now_ms());
    let snapshot = inner.controller.snapshot();
    let retry = match (next, inner.request.clone()) {
        (
            Some(Next::Retry {
                extension,
                delay_ms,
                ..
            }),
            Some(request),
        ) => Some((request.url_for(extension), delay_ms)),
        _ => None,
    };
    (snapshot, retry)
}

/// Routes a failed [`start_session`] call - initial or retry - through the controller
/// instead of only logging it. Applies the same recoverable-error retry budget as any other
/// playback failure and schedules the next attempt when one is due; otherwise this
/// generation is left in a terminal `Failed` state (never a `Recovering`/`Connecting` state
/// with no live work and nothing scheduled).
///
/// Device-open failures (the only way [`start_session`] fails) are treated as transient:
/// a device can be temporarily busy or briefly unavailable, and the existing bounded budget
/// already stops this from retrying forever.
fn handle_startup_failure(
    app: &AppHandle,
    state: &State<'_, PlayerState>,
    generation: Generation,
    attempt: u64,
    message: String,
) -> Snapshot {
    let (snapshot, retry) = {
        let mut inner = state.inner.lock().expect("player state poisoned");
        if inner.active_attempt != Some(attempt) {
            return inner.controller.snapshot();
        }
        route_startup_failure(&mut inner, generation, message)
    };
    if let Some((url, delay_ms)) = retry {
        schedule_retry(app.clone(), generation, attempt, url, delay_ms);
    }
    snapshot
}

#[tauri::command]
pub async fn player_play(
    app: AppHandle,
    state: State<'_, PlayerState>,
    request: StationRequest,
) -> Result<Snapshot, String> {
    // Take the previous session out under the lock, but drop it outside: teardown joins
    // threads and must never happen while holding the state lock.
    let (previous, generation, attempt, url, settings, device, network, snapshot) = {
        let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
        let previous = inner.session.take();
        let generation = inner.controller.play(request.station_id.clone(), now_ms());
        let attempt = inner.begin_attempt();
        let url = request.url_for(Controller::extension_for_attempt(0));
        inner.request = Some(request);
        (
            previous,
            generation,
            attempt,
            url,
            inner.settings.clone(),
            inner.device.clone(),
            state.network.clone(),
            inner.controller.snapshot(),
        )
    };
    drop(previous);

    let sink = Arc::new(TauriSink {
        app: app.clone(),
        attempt,
    }) as Arc<dyn EventSink>;
    match start_session(generation, url, &device, settings, network, sink) {
        Ok(session) => {
            let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
            if inner.accepts_attempt(generation, attempt) {
                // Track the device actually opened, which may differ from the one
                // requested when a saved device has disappeared.
                inner.controller.set_device(session.device_name());
                inner.session = Some(session);
            }
            Ok(snapshot)
        }
        // A synchronous device-open failure is not a special case: route it through the
        // same retry/give-up policy as every other playback failure, so the frontend always
        // sees a coherent snapshot (never a bare invoke rejection that bypasses the revision
        // guard - see `handle_startup_failure` and `playerStore.ts`'s snapshot-only rule).
        Err(e) => Ok(handle_startup_failure(&app, &state, generation, attempt, e)),
    }
}

#[tauri::command]
pub async fn player_stop(state: State<'_, PlayerState>) -> Result<Snapshot, String> {
    let (previous, snapshot) = {
        let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
        let previous = inner.session.take();
        inner.controller.stop();
        inner.active_attempt = None;
        inner.request = None;
        (previous, inner.controller.snapshot())
    };
    drop(previous);
    Ok(snapshot)
}

#[tauri::command]
pub async fn player_set_volume(state: State<'_, PlayerState>, volume: u8) -> Result<(), String> {
    let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
    inner.settings.volume = volume.min(100);
    let settings = inner.settings.clone();
    if let Some(session) = inner.session.as_ref() {
        session.update_settings(settings);
    }
    Ok(())
}

#[tauri::command]
pub async fn player_set_muted(state: State<'_, PlayerState>, muted: bool) -> Result<(), String> {
    let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
    inner.settings.muted = muted;
    let settings = inner.settings.clone();
    if let Some(session) = inner.session.as_ref() {
        session.update_settings(settings);
    }
    Ok(())
}

#[tauri::command]
pub async fn player_set_equalizer(
    state: State<'_, PlayerState>,
    enabled: bool,
    gains: Vec<f64>,
) -> Result<(), String> {
    if gains.len() != apogee_playback_core::dsp::BANDS.len() {
        return Err(format!(
            "equalizer requires exactly {} bands",
            apogee_playback_core::dsp::BANDS.len()
        ));
    }
    let mut array = [0.0f64; 10];
    array.copy_from_slice(&gains);

    let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
    inner.settings.equalizer_enabled = enabled;
    inner.settings.equalizer_gains = array;
    let settings = inner.settings.clone();
    if let Some(session) = inner.session.as_ref() {
        session.update_settings(settings);
    }
    Ok(())
}

#[tauri::command]
pub async fn player_list_devices() -> Result<Vec<DeviceDescriptor>, String> {
    list_output_devices().map_err(|e| e.to_string())
}

/// Select an output device. `None` follows the system default, including later changes.
#[tauri::command]
pub async fn player_set_device(
    app: AppHandle,
    state: State<'_, PlayerState>,
    device_id: Option<String>,
) -> Result<(), String> {
    let request = match device_id {
        Some(id) if !id.is_empty() => DeviceRequest::Specific(id),
        _ => DeviceRequest::SystemDefault,
    };
    restart_for_device(&app, &state, request).map(|_| ())
}

/// Restarts the active station against a newly selected output. A new controller generation
/// invalidates every late event from the old source/output before teardown begins.
fn restart_for_device(
    app: &AppHandle,
    state: &PlayerState,
    device: DeviceRequest,
) -> Result<Snapshot, String> {
    let restart = {
        let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
        inner.device = device.clone();
        let Some(request) = inner.request.clone() else {
            return Ok(inner.controller.snapshot());
        };
        let previous = inner.session.take();
        let generation = inner.controller.play(request.station_id.clone(), now_ms());
        let attempt = inner.begin_attempt();
        let url = request.url_for(Controller::extension_for_attempt(0));
        let snapshot = inner.controller.snapshot();
        Some((
            previous,
            generation,
            attempt,
            url,
            inner.settings.clone(),
            state.network.clone(),
            snapshot,
        ))
    };

    let Some((previous, generation, attempt, url, settings, network, initial_snapshot)) = restart
    else {
        unreachable!();
    };
    drop(previous);
    let _ = app.emit("player-snapshot", initial_snapshot);

    let sink = Arc::new(TauriSink {
        app: app.clone(),
        attempt,
    }) as Arc<dyn EventSink>;
    match start_session(generation, url, &device, settings, network, sink) {
        Ok(session) => {
            let snapshot = {
                let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
                if inner.accepts_attempt(generation, attempt) {
                    inner.controller.set_device(session.device_name());
                    inner.session = Some(session);
                }
                inner.controller.snapshot()
            };
            let _ = app.emit("player-snapshot", snapshot.clone());
            Ok(snapshot)
        }
        Err(error) => {
            let message = error.clone();
            let state = app.state::<PlayerState>();
            let snapshot = handle_startup_failure(app, &state, generation, attempt, error);
            let _ = app.emit("player-snapshot", snapshot);
            Err(message)
        }
    }
}

/// Polls the effective system default outside the callback. CPAL does not expose a portable
/// default-device notification API; a bounded poll keeps this cross-platform and only
/// reopens playback when the user explicitly chose "System default" and its id changed.
pub fn start_default_device_watcher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let state = app.state::<PlayerState>();
            let current = {
                let inner = match state.inner.lock() {
                    Ok(inner) => inner,
                    Err(_) => return,
                };
                if inner.device != DeviceRequest::SystemDefault {
                    None
                } else {
                    inner.session.as_ref().and_then(Session::device_id)
                }
            };
            let new_default = current.as_ref().and_then(|_| {
                list_output_devices()
                    .ok()
                    .and_then(|devices| devices.into_iter().find(|d| d.is_default))
                    .map(|d| d.id)
            });
            let should_reopen = new_default.is_some() && new_default != current;
            if should_reopen {
                if let Err(e) = restart_for_device(&app, &state, DeviceRequest::SystemDefault) {
                    log::warn!("could not follow changed system audio device: {e}");
                }
            }
        }
    });
}

/// Result of migrating a saved MPV device identifier.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceMigration {
    /// The CPAL device id to persist, or `None` to follow the system default.
    ///
    /// The caller **must** persist this. Returning only an explanation would leave the
    /// engine using a migrated device while settings still said "system default", so the
    /// migration would silently undo itself on the next launch.
    pub device_id: Option<String>,
    /// Why the stored value could not be honoured, when it could not.
    pub notice: Option<String>,
}

/// Migrate a saved MPV device identifier to a CPAL device.
#[tauri::command]
pub async fn player_migrate_device(
    state: State<'_, PlayerState>,
    stored: Option<String>,
) -> Result<DeviceMigration, String> {
    let available = list_output_devices().unwrap_or_default();
    let (request, notice) = migrate_mpv_device(stored.as_deref(), &available);
    let device_id = match &request {
        DeviceRequest::Specific(id) => Some(id.clone()),
        DeviceRequest::SystemDefault => None,
    };
    let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
    inner.device = request;
    Ok(DeviceMigration { device_id, notice })
}

/// Enable or disable the spectrum visualiser. Disabled means no FFT work is performed.
#[tauri::command]
pub async fn player_set_visualizer(
    state: State<'_, PlayerState>,
    enabled: bool,
) -> Result<(), String> {
    let mut inner = state.inner.lock().map_err(|_| "player state poisoned")?;
    inner.settings.visualizer_enabled = enabled;
    let settings = inner.settings.clone();
    if let Some(session) = inner.session.as_ref() {
        session.update_settings(settings);
    }
    Ok(())
}

#[tauri::command]
pub async fn player_get_snapshot(state: State<'_, PlayerState>) -> Result<Snapshot, String> {
    let inner = state.inner.lock().map_err(|_| "player state poisoned")?;
    Ok(inner.controller.snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> StationRequest {
        StationRequest {
            base_url: "https://example.invalid/".to_string(),
            username: "user name".to_string(),
            password: "p@ss/word".to_string(),
            stream_id: 1_080_037,
            station_id: "1080037".to_string(),
        }
    }

    /// Pins the exact JSON `src/lib/playerClient.ts` sends. A rename on either side
    /// breaks this rather than failing at runtime with "missing field".
    #[test]
    fn the_request_deserialises_from_the_json_the_frontend_actually_sends() {
        let wire = serde_json::json!({
            "baseUrl": "https://example.invalid",
            "username": "alice",
            "password": "secret",
            "streamId": 1_080_037_i64,
            "stationId": "1080037",
        });
        let parsed: StationRequest =
            serde_json::from_value(wire).expect("frontend JSON must deserialise");
        assert_eq!(parsed.base_url, "https://example.invalid");
        assert_eq!(parsed.username, "alice");
        assert_eq!(parsed.stream_id, 1_080_037);
        assert_eq!(parsed.station_id, "1080037");
    }

    #[test]
    fn snake_case_field_names_are_rejected_so_the_contract_stays_unambiguous() {
        let wire = serde_json::json!({
            "base_url": "https://example.invalid",
            "username": "alice",
            "password": "secret",
            "stream_id": 1_i64,
            "station_id": "1",
        });
        assert!(
            serde_json::from_value::<StationRequest>(wire).is_err(),
            "only the camelCase wire shape is supported"
        );
    }

    #[test]
    fn stream_urls_preserve_the_provider_path_shape() {
        let r = request();
        assert_eq!(
            r.url_for(".ts"),
            "https://example.invalid/live/user%20name/p%40ss%2Fword/1080037.ts"
        );
    }

    #[test]
    fn credentials_are_percent_encoded_so_they_cannot_change_the_url_structure() {
        let r = request();
        let url = r.url_for(".ts");
        // A raw slash in the password would otherwise invent an extra path segment.
        assert!(
            !url.contains("p@ss/word"),
            "credentials must be escaped: {url}"
        );
        assert_eq!(
            url.matches('/').count(),
            6,
            "unexpected path depth in {url}"
        );
    }

    #[test]
    fn a_trailing_slash_on_the_base_url_does_not_double_up() {
        let mut r = request();
        r.base_url = "https://example.invalid///".to_string();
        assert!(r
            .url_for(".ts")
            .starts_with("https://example.invalid/live/"));
    }

    #[test]
    fn both_extension_candidates_are_reachable_from_the_same_request() {
        let r = request();
        assert!(r
            .url_for(Controller::extension_for_attempt(0))
            .ends_with(".ts"));
        assert!(r
            .url_for(Controller::extension_for_attempt(1))
            .ends_with(".m3u8"));
    }

    // -----------------------------------------------------------------
    // Finding 4: a failed `start_session` call (initial or retry) must be routed through
    // the controller's retry/give-up policy, never just logged and abandoned.
    //
    // `route_startup_failure` is the pure decision step behind `handle_startup_failure`
    // (and, transitively, both `player_play`'s and `schedule_retry`'s failure branches) -
    // exercised directly here against a plain `Inner`, with no Tauri app required.
    // -----------------------------------------------------------------

    use apogee_playback_core::session::{PlaybackState, MAX_CONNECT_ATTEMPTS};

    fn fresh_inner() -> (Inner, Generation) {
        let mut controller = Controller::new();
        let generation = controller.play("station-a", 0);
        let inner = Inner {
            controller,
            session: None,
            settings: AudioSettings::default(),
            device: DeviceRequest::default(),
            request: Some(request()),
            attempt_serial: 1,
            active_attempt: Some(1),
        };
        (inner, generation)
    }

    #[test]
    fn a_new_retry_attempt_invalidates_late_events_from_the_previous_attempt() {
        let (mut inner, generation) = fresh_inner();
        let first = inner.active_attempt.unwrap();
        assert!(inner.accepts_attempt(generation, first));

        let retry = inner.begin_attempt();
        assert_ne!(first, retry);
        assert!(!inner.accepts_attempt(generation, first));
        assert!(inner.accepts_attempt(generation, retry));
    }

    #[test]
    fn stopping_invalidates_the_active_attempt_even_when_the_generation_matches() {
        let (mut inner, generation) = fresh_inner();
        let attempt = inner.active_attempt.unwrap();
        inner.controller.stop();
        inner.active_attempt = None;
        assert!(!inner.accepts_attempt(generation, attempt));
    }

    #[test]
    fn a_startup_failure_schedules_a_retry_instead_of_only_logging() {
        let (mut inner, generation) = fresh_inner();
        let (snapshot, retry) =
            route_startup_failure(&mut inner, generation, "device busy".to_string());

        assert_eq!(
            snapshot.state,
            PlaybackState::Recovering,
            "a recoverable device failure must not be a dead end"
        );
        let (url, _delay_ms) = retry.expect(
            "previously a start_session failure was only logged, leaving Recovering with no \
             scheduled retry - this must now schedule one",
        );
        assert!(
            url.ends_with(".m3u8"),
            "the next attempt should alternate the extension: {url}"
        );
    }

    #[test]
    fn repeated_startup_failures_keep_retrying_within_the_attempt_budget() {
        // `route_startup_failure` uses the real wall clock (via `now_ms`), so a fast test
        // cannot force the *time* budget to exhaust - that half of the give-up condition is
        // already covered independently of wall-clock speed by
        // `playback-core/tests/session.rs` (same `Controller::on_error` this delegates to).
        // What belongs here is `commands.rs`'s own plumbing: that consecutive startup
        // failures keep producing a scheduled retry - never silently stopping - for as long
        // as the attempt budget allows, exactly the "retry within budget" half of finding 4.
        let (mut inner, generation) = fresh_inner();
        for attempt in 1..MAX_CONNECT_ATTEMPTS {
            let (snapshot, retry) =
                route_startup_failure(&mut inner, generation, "device busy".to_string());
            assert_eq!(
                snapshot.state,
                PlaybackState::Recovering,
                "attempt {attempt}: should still be recovering within the attempt budget"
            );
            assert!(
                retry.is_some(),
                "attempt {attempt}: a startup failure must schedule another attempt \
                 instead of only being logged"
            );
        }
    }

    #[test]
    fn a_permanent_looking_error_class_is_never_produced_for_a_device_failure() {
        // `route_startup_failure` always classifies a `start_session` failure as
        // transient (a device can become available again) - it must never give up after a
        // single attempt the way a permanent/credentials error would.
        let (mut inner, generation) = fresh_inner();
        let (snapshot, retry) =
            route_startup_failure(&mut inner, generation, "device busy".to_string());
        assert_ne!(
            snapshot.state,
            PlaybackState::Failed,
            "a single device-open failure must not be treated as terminal"
        );
        assert!(retry.is_some());
    }

    #[test]
    fn a_device_recovering_within_the_budget_can_still_succeed() {
        // Mirrors the "retry within budget then succeed" acceptance criterion: a couple of
        // failures must still leave room to retry, not immediately give up.
        let (mut inner, generation) = fresh_inner();
        let (first, retry) =
            route_startup_failure(&mut inner, generation, "device busy".to_string());
        assert_eq!(first.state, PlaybackState::Recovering);
        assert!(retry.is_some());

        // The device becomes available on the very next attempt: nothing more to route
        // here (a successful `start_session` is handled by `schedule_retry`'s Ok branch,
        // outside this pure function), but the controller must still have budget left.
        assert!(inner.controller.attempt() < MAX_CONNECT_ATTEMPTS);
    }

    #[test]
    fn a_startup_failure_for_a_stale_generation_is_ignored() {
        let (mut inner, generation) = fresh_inner();
        // The user switched stations (or stopped) before this attempt's failure was
        // reported back - e.g. during the gap between `start_session` failing and the
        // lock being reacquired.
        let newer = inner.controller.play("station-b", 0);
        assert_ne!(generation, newer);

        let (snapshot, retry) =
            route_startup_failure(&mut inner, generation, "device busy".to_string());
        assert!(
            retry.is_none(),
            "a stale attempt must never schedule a retry that could install a session for \
             an old generation"
        );
        assert_eq!(
            snapshot.generation, newer,
            "the stale failure must not have touched the current generation's state"
        );
        assert_eq!(
            snapshot.state,
            PlaybackState::Connecting,
            "the newer station's own connecting state must be untouched by the old attempt"
        );
    }

    #[test]
    fn a_startup_failure_with_no_pending_request_never_schedules_a_retry() {
        // Defensive: if `request` were ever `None` (should not normally happen once
        // `player_play` has run), there is no URL to retry with - never panic or schedule
        // a retry to nowhere.
        let mut controller = Controller::new();
        let generation = controller.play("station-a", 0);
        let mut inner = Inner {
            controller,
            session: None,
            settings: AudioSettings::default(),
            device: DeviceRequest::default(),
            request: None,
            attempt_serial: 1,
            active_attempt: Some(1),
        };
        let (_snapshot, retry) =
            route_startup_failure(&mut inner, generation, "device busy".to_string());
        assert!(retry.is_none());
    }
}
