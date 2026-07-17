use super::{CaptureSource, SelectedAudioDevice};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::io::{self, ErrorKind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Opens a real-time system-audio-loopback capture on Windows via cpal's
/// WASAPI backend: passing the *default output* device to
/// `build_input_stream` transparently sets `AUDCLNT_STREAMFLAGS_LOOPBACK`,
/// so no raw WASAPI/COM bindings are needed here.
pub(super) async fn open_windows_capture_source(
    selected: Option<SelectedAudioDevice>,
) -> io::Result<CaptureSource> {
    let host = cpal::default_host();
    // Loopback captures whatever the *output* device is playing, so to follow
    // the user's selection the capture device must be that same output device.
    // cpal identifies devices by their friendly name, which matches mpv's
    // `description`; fall back to the default output for "system default" or if
    // the named device isn't currently present.
    let device = selected
        .as_ref()
        .and_then(|dev| {
            host.output_devices().ok().and_then(|mut devices| {
                devices.find(|d| d.name().ok().as_deref() == Some(dev.description.as_str()))
            })
        })
        .or_else(|| host.default_output_device())
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "no audio output device"))?;
    let device_name = device
        .name()
        .unwrap_or_else(|_| "unknown device".to_string());
    log::info!("waveform: opening WASAPI loopback on output device '{device_name}'");

    let config = device
        .default_output_config()
        .map_err(|e| io::Error::other(format!("failed to read default output config: {e}")))?;
    let sample_rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();

    let (tx, rx) = mpsc::channel::<Vec<f32>>(4);
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<io::Result<()>>();

    std::thread::spawn(move || {
        // cpal's WASAPI `Stream` isn't `Send`, so it must be built, played,
        // and dropped from the same thread it's created on. This thread
        // parks (with a periodic wakeup to check `stop`) for the stream's
        // entire lifetime instead of handing the stream off anywhere else.
        let err_fn_stop = stop.clone();
        let err_fn = move |err: cpal::StreamError| {
            log::warn!("waveform: WASAPI loopback stream error ({err}), stopping capture");
            err_fn_stop.store(true, Ordering::SeqCst);
        };

        let stream_result = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                &stream_config,
                {
                    let tx = tx.clone();
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        send_downmixed(&tx, data, channels, |s| s)
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                &stream_config,
                {
                    let tx = tx.clone();
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        send_downmixed(&tx, data, channels, |s| s as f32 / 32768.0)
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::U16 => device.build_input_stream(
                &stream_config,
                {
                    let tx = tx.clone();
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        send_downmixed(&tx, data, channels, |s| (s as f32 - 32768.0) / 32768.0)
                    }
                },
                err_fn,
                None,
            ),
            other => {
                let _ = ready_tx.send(Err(io::Error::new(
                    ErrorKind::Unsupported,
                    format!("unsupported WASAPI sample format {other:?}"),
                )));
                return;
            }
        };

        let stream = match stream_result {
            Ok(stream) => stream,
            Err(e) => {
                let _ = ready_tx.send(Err(io::Error::other(format!(
                    "failed to build WASAPI loopback stream: {e}"
                ))));
                return;
            }
        };

        if let Err(e) = stream.play() {
            let _ = ready_tx.send(Err(io::Error::other(format!(
                "failed to start WASAPI loopback stream: {e}"
            ))));
            return;
        }

        let _ = ready_tx.send(Ok(()));

        while !stop.load(Ordering::SeqCst) {
            std::thread::park_timeout(Duration::from_millis(250));
        }
        // `stream` (and its data-callback's clone of `tx`) drops here,
        // stopping capture and closing the channel so `process_samples`
        // sees the source end and the retry loop reopens a fresh one.
        drop(stream);
        drop(tx);
    });

    ready_rx
        .recv()
        .map_err(|_| io::Error::other("WASAPI capture thread ended before starting"))??;

    Ok(CaptureSource {
        rx,
        sample_rate,
        backend_name: "WASAPI loopback",
    })
}

fn send_downmixed<T: Copy>(
    tx: &mpsc::Sender<Vec<f32>>,
    data: &[T],
    channels: usize,
    to_f32: impl Fn(T) -> f32,
) {
    if channels == 0 {
        return;
    }
    let mono: Vec<f32> = data
        .chunks_exact(channels)
        .map(|frame| frame.iter().map(|&s| to_f32(s)).sum::<f32>() / channels as f32)
        .collect();
    let _ = tx.blocking_send(mono);
}
