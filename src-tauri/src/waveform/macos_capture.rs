use super::CaptureSource;
use objc2::rc::Retained;
use objc2::AllocAnyThread;
use objc2_core_audio::{
    kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceNameKey,
    kAudioAggregateDeviceTapAutoStartKey, kAudioAggregateDeviceTapListKey,
    kAudioAggregateDeviceUIDKey, kAudioDevicePropertyNominalSampleRate,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioSubTapUIDKey,
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProc, AudioDeviceIOProcID,
    AudioDeviceStart, AudioDeviceStop, AudioHardwareCreateAggregateDevice,
    AudioHardwareCreateProcessTap, AudioHardwareDestroyAggregateDevice,
    AudioHardwareDestroyProcessTap, AudioObjectGetPropertyData, AudioObjectID,
    AudioObjectPropertyAddress, CATapDescription,
};
use objc2_core_foundation::{
    CFArray, CFBoolean, CFMutableDictionary, CFRetained, CFString, CFType,
};
use objc2_foundation::{NSArray, NSNumber, NSString};
use std::ffi::c_void;
use std::io::{self, ErrorKind};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Apple only added true system-audio loopback (the Core Audio Process Tap
/// API) in macOS 14.2. Older systems keep the frontend's synthetic fallback
/// animation, same as when no backend is available at all.
fn macos_supports_process_tap() -> bool {
    match macos_os_version() {
        Some((major, minor)) => (major, minor) >= (14, 2),
        None => false,
    }
}

fn macos_os_version() -> Option<(u32, u32)> {
    let name = std::ffi::CString::new("kern.osproductversion").ok()?;
    let mut buf = [0u8; 32];
    let mut len = buf.len();
    let ret = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr() as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if ret != 0 {
        return None;
    }
    let s = std::str::from_utf8(&buf[..len.saturating_sub(1)]).ok()?;
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor))
}

/// A mono, global tap excluding no processes - i.e. everything currently
/// playing through the default output device.
fn build_tap_description() -> Retained<CATapDescription> {
    let empty_processes: Retained<NSArray<NSNumber>> = NSArray::new();
    unsafe {
        let desc = CATapDescription::alloc();
        CATapDescription::initMonoGlobalTapButExcludeProcesses(desc, &empty_processes)
    }
}

fn create_process_tap(desc: &CATapDescription) -> io::Result<AudioObjectID> {
    let mut tap_id: AudioObjectID = 0;
    let status =
        unsafe { AudioHardwareCreateProcessTap(Some(desc), &mut tap_id as *mut AudioObjectID) };
    if status != 0 {
        return Err(io::Error::other(format!(
            "AudioHardwareCreateProcessTap failed: {status}"
        )));
    }
    Ok(tap_id)
}

fn cf_key(key: &std::ffi::CStr) -> CFRetained<CFString> {
    CFString::from_str(
        key.to_str()
            .expect("CoreAudio property keys are always valid UTF-8"),
    )
}

/// Wraps the process tap as the sole input of a private aggregate device -
/// installing an IOProc directly on a bare tap isn't supported, so every
/// Core Audio tap consumer (Apple's own AudioCap sample, the `audiotee`
/// CLI) goes through this aggregate-device indirection.
fn create_aggregate_device(
    tap_uid: &NSString,
    agg_uid: &str,
    name: &str,
) -> io::Result<AudioObjectID> {
    let sub_tap_dict: CFRetained<CFMutableDictionary<CFString, CFType>> =
        CFMutableDictionary::empty();
    let tap_uid_cf = CFString::from_str(&tap_uid.to_string());
    sub_tap_dict.set(&cf_key(kAudioSubTapUIDKey), tap_uid_cf.as_ref());

    let composition: CFRetained<CFMutableDictionary<CFString, CFType>> =
        CFMutableDictionary::empty();
    let name_cf = CFString::from_str(name);
    let uid_cf = CFString::from_str(agg_uid);
    let tap_list = CFArray::from_retained_objects(&[sub_tap_dict]);
    composition.set(&cf_key(kAudioAggregateDeviceNameKey), name_cf.as_ref());
    composition.set(&cf_key(kAudioAggregateDeviceUIDKey), uid_cf.as_ref());
    composition.set(&cf_key(kAudioAggregateDeviceTapListKey), tap_list.as_ref());
    composition.set(
        &cf_key(kAudioAggregateDeviceIsPrivateKey),
        CFBoolean::new(true).as_ref(),
    );
    composition.set(
        &cf_key(kAudioAggregateDeviceTapAutoStartKey),
        CFBoolean::new(true).as_ref(),
    );

    // `AudioHardwareCreateAggregateDevice` takes a type-erased `CFDictionary`
    // (its `K`/`V` are `Opaque`, which can't satisfy the `Type + PartialEq +
    // Hash` bounds needed to *build* a dictionary above) - the underlying
    // `CFDictionaryRef` is identical regardless of the Rust-side key/value
    // phantom types, so this is exactly what `cast_unchecked` is for.
    let composition: CFRetained<objc2_core_foundation::CFDictionary> =
        unsafe { CFRetained::cast_unchecked(composition) };
    let mut device_id: AudioObjectID = 0;
    let status = unsafe {
        AudioHardwareCreateAggregateDevice(composition.as_ref(), NonNull::from(&mut device_id))
    };
    if status != 0 {
        return Err(io::Error::other(format!(
            "AudioHardwareCreateAggregateDevice failed: {status}"
        )));
    }
    Ok(device_id)
}

fn nominal_sample_rate(device_id: AudioObjectID) -> io::Result<u32> {
    let address = AudioObjectPropertyAddress {
        mSelector: kAudioDevicePropertyNominalSampleRate,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };
    let mut rate: f64 = 0.0;
    let mut size = std::mem::size_of::<f64>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            device_id,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut rate).cast::<c_void>(),
        )
    };
    if status != 0 || rate <= 0.0 {
        return Err(io::Error::other(format!(
            "failed to read aggregate device sample rate: status {status}"
        )));
    }
    Ok(rate.round() as u32)
}

/// Downmixes an interleaved Float32 `AudioBufferList` (the format Core Audio
/// process taps deliver) to mono and forwards it to `process_samples` via
/// the channel stashed in `client_data`.
unsafe extern "C-unwind" fn io_proc(
    _device: AudioObjectID,
    _now: NonNull<objc2_core_audio_types::AudioTimeStamp>,
    input_data: NonNull<objc2_core_audio_types::AudioBufferList>,
    _input_time: NonNull<objc2_core_audio_types::AudioTimeStamp>,
    _output_data: NonNull<objc2_core_audio_types::AudioBufferList>,
    _output_time: NonNull<objc2_core_audio_types::AudioTimeStamp>,
    client_data: *mut c_void,
) -> i32 {
    let tx = unsafe { &*(client_data as *const mpsc::Sender<Vec<f32>>) };
    let buffer_list = unsafe { input_data.as_ref() };
    if buffer_list.mNumberBuffers == 0 {
        return 0;
    }
    let buffer = &buffer_list.mBuffers[0];
    let channels = buffer.mNumberChannels.max(1) as usize;
    let sample_count = buffer.mDataByteSize as usize / std::mem::size_of::<f32>();
    if buffer.mData.is_null() || sample_count == 0 {
        return 0;
    }
    let samples = unsafe { std::slice::from_raw_parts(buffer.mData as *const f32, sample_count) };
    let mono: Vec<f32> = samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    let _ = tx.blocking_send(mono);
    0
}

struct TeardownHandles {
    tap_id: AudioObjectID,
    device_id: AudioObjectID,
    proc_id: AudioDeviceIOProcID,
    client_data: *mut c_void,
}

fn teardown(handles: TeardownHandles) {
    unsafe {
        let _ = AudioDeviceStop(handles.device_id, handles.proc_id);
        let _ = AudioDeviceDestroyIOProcID(handles.device_id, handles.proc_id);
        drop(Box::from_raw(
            handles.client_data as *mut mpsc::Sender<Vec<f32>>,
        ));
        let _ = AudioHardwareDestroyAggregateDevice(handles.device_id);
        let _ = AudioHardwareDestroyProcessTap(handles.tap_id);
    }
}

pub(super) async fn open_macos_capture_source(
    _selected: Option<super::SelectedAudioDevice>,
) -> io::Result<CaptureSource> {
    // The selected device is intentionally ignored here: this is a *global*
    // process tap (see build_tap_description) that captures all system audio
    // regardless of which output device it's routed to, so it already follows
    // mpv wherever the user points its output. Device selection therefore only
    // needs to change mpv's output on macOS, not this capture.
    if !macos_supports_process_tap() {
        return Err(io::Error::new(
            ErrorKind::Unsupported,
            "Core Audio process taps require macOS 14.2+ (audio-capture permission also required)",
        ));
    }

    let (tx, rx) = mpsc::channel::<Vec<f32>>(4);
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<io::Result<u32>>();

    std::thread::spawn(move || {
        // The tap/aggregate-device objects and the ObjC `CATapDescription`
        // aren't `Send`, so the whole setup, the IOProc's active lifetime,
        // and teardown all happen on this one dedicated thread - the same
        // pattern as the Windows cpal capture thread.
        let desc = build_tap_description();
        let tap_id = match create_process_tap(&desc) {
            Ok(id) => id,
            Err(e) => {
                let _ = ready_tx.send(Err(e));
                return;
            }
        };
        let tap_uid = unsafe { desc.UUID() }.UUIDString();
        let device_id = match create_aggregate_device(
            &tap_uid,
            "com.apogee.waveform-tap",
            "Apogee Waveform Tap",
        ) {
            Ok(id) => id,
            Err(e) => {
                unsafe {
                    let _ = AudioHardwareDestroyProcessTap(tap_id);
                }
                let _ = ready_tx.send(Err(e));
                return;
            }
        };
        let sample_rate = match nominal_sample_rate(device_id) {
            Ok(rate) => rate,
            Err(e) => {
                unsafe {
                    let _ = AudioHardwareDestroyAggregateDevice(device_id);
                    let _ = AudioHardwareDestroyProcessTap(tap_id);
                }
                let _ = ready_tx.send(Err(e));
                return;
            }
        };

        let client_data = Box::into_raw(Box::new(tx)) as *mut c_void;
        let mut proc_id: AudioDeviceIOProcID = None;
        let cb: AudioDeviceIOProc = Some(io_proc);
        let create_status = unsafe {
            AudioDeviceCreateIOProcID(device_id, cb, client_data, NonNull::from(&mut proc_id))
        };
        if create_status != 0 {
            unsafe {
                drop(Box::from_raw(client_data as *mut mpsc::Sender<Vec<f32>>));
                let _ = AudioHardwareDestroyAggregateDevice(device_id);
                let _ = AudioHardwareDestroyProcessTap(tap_id);
            }
            let _ = ready_tx.send(Err(io::Error::other(format!(
                "AudioDeviceCreateIOProcID failed: {create_status}"
            ))));
            return;
        }

        let start_status = unsafe { AudioDeviceStart(device_id, proc_id) };
        if start_status != 0 {
            unsafe {
                let _ = AudioDeviceDestroyIOProcID(device_id, proc_id);
                drop(Box::from_raw(client_data as *mut mpsc::Sender<Vec<f32>>));
                let _ = AudioHardwareDestroyAggregateDevice(device_id);
                let _ = AudioHardwareDestroyProcessTap(tap_id);
            }
            let _ = ready_tx.send(Err(io::Error::other(format!(
                "AudioDeviceStart failed: {start_status}"
            ))));
            return;
        }

        let _ = ready_tx.send(Ok(sample_rate));

        // Unlike the Linux (pipe EOF) and Windows (cpal error callback)
        // sources, there's no device-invalidation listener wired up here
        // (e.g. `kAudioDevicePropertyDeviceIsAlive`), so `stop` never gets
        // set from inside this thread today - a mid-session default-output
        // change won't self-heal on macOS the way it does on the other two
        // platforms. `stop` is kept (rather than parking unconditionally)
        // so that gap can be closed later without changing this loop.
        while !stop.load(Ordering::SeqCst) {
            std::thread::park_timeout(Duration::from_millis(250));
        }
        teardown(TeardownHandles {
            tap_id,
            device_id,
            proc_id,
            client_data,
        });
    });

    let sample_rate = ready_rx
        .recv()
        .map_err(|_| io::Error::other("Core Audio tap thread ended before starting"))??;

    Ok(CaptureSource {
        rx,
        sample_rate,
        backend_name: "Core Audio process tap",
    })
}
