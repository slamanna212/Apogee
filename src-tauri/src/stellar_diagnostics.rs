//! Explicit, bounded comparison run. Never logs request credentials or raw bodies.
use reqwest::{redirect::Policy, Client};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::WebviewWindow;
use tokio_util::sync::CancellationToken;

const URL: &str = "https://api.stellartunerlog.com/v1/nowplaying";
static RUNNING: AtomicBool = AtomicBool::new(false);
struct RunGuard;
impl Drop for RunGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}

fn client(case: &str) -> Result<Client, String> {
    let mut builder = Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        // Fixed endpoint only; never forward the key to a redirected host.
        .redirect(Policy::none())
        .user_agent(if matches!(case, "old-user-agent" | "legacy-combined") {
            "tauri-plugin-http/2.5.9"
        } else {
            concat!("Apogee/", env!("CARGO_PKG_VERSION"))
        });
    if !matches!(case, "http2" | "legacy-combined") {
        builder = builder.http1_only();
    }
    if matches!(case, "cookies" | "legacy-combined") {
        builder = builder.cookie_store(true);
    }
    if case == "direct" {
        builder = builder.no_proxy();
    }
    if case == "ipv4" {
        builder = builder.local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    }
    if case == "ipv6" {
        builder = builder.local_address(std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED));
    }
    builder
        .build()
        .map_err(|_| "Could not construct diagnostic client".to_string())
}

#[tauri::command]
pub async fn stellar_run_diagnostics(window: WebviewWindow) -> Result<(), String> {
    if RUNNING.swap(true, Ordering::AcqRel) {
        return Err("Stellar diagnostics are already running".into());
    }
    let _guard = RunGuard;
    let key = crate::secrets::secrets_get_builtin_stellar_key();
    let key = key.as_deref().filter(|key| !key.is_empty());
    let url = window
        .url()
        .map_err(|_| "Could not determine application origin")?;
    let origin = if url.scheme() == "tauri" {
        "tauri://localhost".to_string()
    } else {
        url.origin().ascii_serialization()
    };
    let cases = [
        "pooled",
        "fresh",
        "old-user-agent",
        "origin",
        "http2",
        "cookies",
        "direct",
        "ipv4",
        "ipv6",
        "legacy-combined",
    ];
    let clients = cases
        .iter()
        .map(|case| client(case))
        .collect::<Result<Vec<_>, _>>()?;
    let headers: Vec<(&str, &str)> = key.map(|key| vec![("X-API-Key", key)]).unwrap_or_default();
    log::info!("stellar diagnostic BEGIN: os={} arch={} key_present={} origin={} rounds=2 system_proxy_feature=false legacy_combined_is_approximation=true",
        std::env::consts::OS, std::env::consts::ARCH, key.is_some(), origin);
    log::info!(
        "stellar diagnostic key_has_outer_whitespace={} key_header_valid={}",
        key.is_some_and(|key| key != key.trim()),
        key.is_none_or(|key| reqwest::header::HeaderValue::from_str(key).is_ok())
    );
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ] {
        log::info!(
            "stellar diagnostic environment {name}_present={}",
            std::env::var_os(name).is_some()
        );
    }
    match tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host(("api.stellartunerlog.com", 443)),
    )
    .await
    {
        Ok(Ok(addresses)) => log::info!(
            "stellar diagnostic DNS ordered_addresses={:?}",
            addresses.collect::<Vec<_>>()
        ),
        _ => log::info!("stellar diagnostic DNS lookup failed or timed out"),
    }
    for round in 0..2 {
        for step in 0..cases.len() {
            let index = if round == 0 {
                step
            } else {
                cases.len() - 1 - step
            };
            let case = cases[index];
            log::info!(
                "stellar diagnostic round={} case={case} starting",
                round + 1
            );
            let fresh;
            let transport = if matches!(case, "fresh" | "legacy-combined") {
                fresh = client(case)?;
                &fresh
            } else {
                &clients[index]
            };
            let mut request = transport.get(URL);
            if let Some(key) = key {
                request = request.header("X-API-Key", key);
            }
            if matches!(case, "origin" | "legacy-combined") {
                request = request.header("Origin", &origin);
            }
            let started = Instant::now();
            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    log::info!("stellar diagnostic round={} case={case} status={} protocol={:?} remote_addr={:?} headers_ms={} headers={}",
                        round + 1, status.as_u16(), response.version(), response.remote_addr(), started.elapsed().as_millis(),
                        crate::network::stellar_response_headers(response.headers(), &headers));
                    match crate::network::read_bounded(response, 2 * 1024 * 1024, &CancellationToken::new()).await {
                        Ok(body) => {
                            let json = serde_json::from_slice::<serde_json::Value>(&body.bytes).ok();
                            let stations = json.as_ref().and_then(|v| v.get("stations")).and_then(|v| v.as_object()).map(|v| v.len());
                            log::info!("stellar diagnostic round={} case={case} body_bytes={} valid_json={} stations={stations:?} total_ms={}", round + 1, body.bytes.len(), json.is_some(), started.elapsed().as_millis());
                        }
                        Err(error) => {
                            let worker_limit = matches!(&error, crate::network::NetworkError::Status { detail: Some(detail), .. } if detail.contains("1102"));
                            log::info!("stellar diagnostic round={} case={case} body_failed=true worker_1102={worker_limit} total_ms={}", round + 1, started.elapsed().as_millis());
                        }
                    }
                }
                Err(error) => log::info!("stellar diagnostic round={} case={case} transport_failed=true timeout={} connect_error={} total_ms={}",
                    round + 1, error.is_timeout(), error.is_connect(), started.elapsed().as_millis()),
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
    log::info!("stellar diagnostic END");
    Ok(())
}
