//! Output device enumeration, selection, and legacy-setting migration.
//!
//! CPAL 0.18 changed shape here: `Device::name()` is gone, replaced by a structured
//! `description()` plus a separate `id()`. `DeviceId`'s own documentation states that
//! applications should persist it via `Display`/`FromStr`, so the saved setting stores that
//! id rather than a display name, which is not unique across devices.
//!
//! Apogee previously stored MPV device identifiers, which are a different namespace
//! entirely. Those are migrated by descriptive match rather than assumed compatible.

use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};

/// How a device is presented to the UI and persisted.
///
/// camelCase on the wire to match the TypeScript caller; without it `is_default` arrives
/// as `undefined` and no device is ever marked as the system default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDescriptor {
    /// Stable identity, persisted. Empty only if the backend could not supply one.
    pub id: String,
    /// Human-readable name. Never assume this is unique.
    pub name: String,
    /// Backend/host, e.g. "Alsa", "Wasapi", "CoreAudio".
    pub backend: String,
    /// Whether this is the system default output.
    pub is_default: bool,
}

/// Which device the user asked for, tracked separately from what is actually in use so a
/// temporary unplug does not erase the preference.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceRequest {
    /// Follow the system default, including when the default changes.
    #[default]
    SystemDefault,
    /// A specific device, by persisted CPAL id.
    Specific(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceError {
    /// No output device exists at all. Recoverable: the user can plug one in.
    NoOutputDevice,
    Backend(String),
}

impl std::fmt::Display for DeviceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoOutputDevice => write!(f, "no audio output device is available"),
            Self::Backend(e) => write!(f, "audio backend error: {e}"),
        }
    }
}

impl std::error::Error for DeviceError {}

fn describe(device: &cpal::Device, backend: &str, default_id: Option<&str>) -> DeviceDescriptor {
    let id = device.id().map(|i| i.to_string()).unwrap_or_default();
    let name = device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|_| "Unknown device".to_string());
    let is_default = default_id.is_some_and(|d| d == id) && !id.is_empty();
    DeviceDescriptor {
        id,
        name,
        backend: backend.to_string(),
        is_default,
    }
}

/// Enumerate output devices on the default host.
///
/// A device that fails to describe itself is skipped rather than failing the whole list:
/// one broken device must not make the picker unusable.
pub fn list_output_devices() -> Result<Vec<DeviceDescriptor>, DeviceError> {
    let host = cpal::default_host();
    let backend = format!("{:?}", host.id());
    let default_id = host
        .default_output_device()
        .and_then(|d| d.id().ok())
        .map(|i| i.to_string());

    let devices = host
        .output_devices()
        .map_err(|e| DeviceError::Backend(e.to_string()))?;

    let mut out = Vec::new();
    for device in devices {
        let descriptor = describe(&device, &backend, default_id.as_deref());
        if descriptor.id.is_empty() {
            continue;
        }
        out.push(descriptor);
    }
    Ok(out)
}

/// Resolve a request to an actual device.
///
/// Falls back to the system default when a specific device has disappeared, reporting which
/// device was actually opened so the caller can tell the user without discarding their
/// stored preference.
pub fn resolve(request: &DeviceRequest) -> Result<(cpal::Device, DeviceDescriptor), DeviceError> {
    let host = cpal::default_host();
    let backend = format!("{:?}", host.id());
    let default_device = host.default_output_device();
    let default_id = default_device
        .as_ref()
        .and_then(|d| d.id().ok())
        .map(|i| i.to_string());

    if let DeviceRequest::Specific(wanted) = request {
        let devices = host
            .output_devices()
            .map_err(|e| DeviceError::Backend(e.to_string()))?;
        for device in devices {
            if device.id().ok().map(|i| i.to_string()).as_deref() == Some(wanted.as_str()) {
                let descriptor = describe(&device, &backend, default_id.as_deref());
                return Ok((device, descriptor));
            }
        }
        // Deliberately fall through to the default rather than failing outright.
    }

    let device = default_device.ok_or(DeviceError::NoOutputDevice)?;
    let descriptor = describe(&device, &backend, default_id.as_deref());
    Ok((device, descriptor))
}

/// Migrate a stored MPV device identifier to a CPAL request.
///
/// MPV identifiers look like `alsa/default`, `pulse/<sink>`, `wasapi/<guid>` or
/// `coreaudio/<uid>`; CPAL ids are a different namespace. Only a confident match is
/// accepted. Anything ambiguous returns `SystemDefault`, which is the safe outcome: the
/// user hears audio from the system default and can re-pick, rather than getting silence
/// from a device that does not exist.
#[must_use]
pub fn migrate_mpv_device(
    stored: Option<&str>,
    available: &[DeviceDescriptor],
) -> (DeviceRequest, Option<String>) {
    let Some(stored) = stored.map(str::trim).filter(|s| !s.is_empty()) else {
        return (DeviceRequest::SystemDefault, None);
    };

    // MPV's own way of saying "system default".
    if stored == "auto" || stored.ends_with("/default") || stored == "default" {
        return (DeviceRequest::SystemDefault, None);
    }

    // Strip the MPV backend prefix; the remainder is the device-specific part.
    let tail = stored.split_once('/').map_or(stored, |(_, rest)| rest);
    let tail_norm = normalize(tail);
    if tail_norm.is_empty() {
        return (DeviceRequest::SystemDefault, None);
    }

    // An exact id match is the only unambiguous outcome.
    if let Some(hit) = available.iter().find(|d| normalize(&d.id) == tail_norm) {
        return (DeviceRequest::Specific(hit.id.clone()), None);
    }

    // Otherwise accept a descriptive match only when exactly one device matches.
    let mut hits = available.iter().filter(|d| {
        normalize(&d.name).contains(&tail_norm) || tail_norm.contains(&normalize(&d.name))
    });
    match (hits.next(), hits.next()) {
        (Some(only), None) => (DeviceRequest::Specific(only.id.clone()), None),
        (Some(_), Some(_)) => (
            DeviceRequest::SystemDefault,
            Some(format!(
                "Your saved audio device ({stored}) matched more than one device, so the system default is being used."
            )),
        ),
        _ => (
            DeviceRequest::SystemDefault,
            Some(format!(
                "Your saved audio device ({stored}) could not be matched, so the system default is being used."
            )),
        ),
    }
}

fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(id: &str, name: &str) -> DeviceDescriptor {
        DeviceDescriptor {
            id: id.to_string(),
            name: name.to_string(),
            backend: "Alsa".to_string(),
            is_default: false,
        }
    }

    /// Pins the wire shape the device picker in `Settings.tsx` reads.
    #[test]
    fn descriptors_serialise_in_the_shape_the_frontend_reads() {
        let d = DeviceDescriptor {
            id: "alsa:default".into(),
            name: "Default Audio Device".into(),
            backend: "Alsa".into(),
            is_default: true,
        };
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["isDefault"], serde_json::json!(true), "got {json}");
        assert_eq!(json["id"], serde_json::json!("alsa:default"));
        assert_eq!(json["name"], serde_json::json!("Default Audio Device"));
        assert_eq!(json["backend"], serde_json::json!("Alsa"));
        assert!(
            json.get("is_default").is_none(),
            "snake_case must not leak: {json}"
        );
    }

    // --- Migration of stored MPV identifiers (pure logic, no hardware) ---

    #[test]
    fn an_absent_or_blank_setting_follows_the_system_default() {
        for stored in [None, Some(""), Some("   ")] {
            let (request, note) = migrate_mpv_device(stored, &[]);
            assert_eq!(request, DeviceRequest::SystemDefault, "{stored:?}");
            assert!(note.is_none(), "{stored:?} should not warn");
        }
    }

    #[test]
    fn mpv_default_spellings_map_to_the_system_default_without_warning() {
        let available = [descriptor("alsa:default", "Default ALSA Output")];
        for stored in ["auto", "default", "alsa/default", "pulse/default"] {
            let (request, note) = migrate_mpv_device(Some(stored), &available);
            assert_eq!(request, DeviceRequest::SystemDefault, "{stored}");
            assert!(
                note.is_none(),
                "{stored} is a normal default, not a failure"
            );
        }
    }

    #[test]
    fn a_uniquely_matching_device_is_migrated() {
        let available = [
            descriptor("alsa:pipewire", "PipeWire Sound Server"),
            descriptor("alsa:null", "Discard all samples"),
        ];
        let (request, note) = migrate_mpv_device(Some("alsa/pipewire"), &available);
        assert_eq!(request, DeviceRequest::Specific("alsa:pipewire".into()));
        assert!(note.is_none(), "a confident match needs no explanation");
    }

    #[test]
    fn an_unmatched_device_falls_back_and_explains_itself() {
        let available = [descriptor("alsa:pipewire", "PipeWire Sound Server")];
        let (request, note) = migrate_mpv_device(Some("wasapi/{some-guid}"), &available);
        assert_eq!(
            request,
            DeviceRequest::SystemDefault,
            "silence would be worse than the default"
        );
        let note = note.expect("the user must be told why their choice was dropped");
        assert!(
            note.contains("wasapi"),
            "the message should name the stored device: {note}"
        );
    }

    #[test]
    fn an_ambiguous_match_refuses_to_guess() {
        // Two identically-named devices: the plan warns display names are not unique.
        let available = [
            descriptor("alsa:usb-1", "USB Audio"),
            descriptor("alsa:usb-2", "USB Audio"),
        ];
        let (request, note) = migrate_mpv_device(Some("alsa/USB Audio"), &available);
        assert_eq!(request, DeviceRequest::SystemDefault);
        assert!(note.is_some_and(|n| n.contains("more than one")));
    }

    #[test]
    fn migration_ignores_punctuation_and_case_differences() {
        let available = [descriptor("alsa:pipewire", "PipeWire Sound Server")];
        let (request, _) = migrate_mpv_device(Some("pulse/PipeWire_Sound-Server"), &available);
        assert_eq!(request, DeviceRequest::Specific("alsa:pipewire".into()));
    }

    // --- Real hardware. Skipped rather than failed where no device exists,
    //     so the suite still runs on a headless CI machine. ---

    #[test]
    fn enumeration_reports_real_devices_with_stable_ids() {
        let Ok(devices) = list_output_devices() else {
            eprintln!("skipping: no audio backend on this machine");
            return;
        };
        if devices.is_empty() {
            eprintln!("skipping: no output devices on this machine");
            return;
        }
        for d in &devices {
            assert!(
                !d.id.is_empty(),
                "a listed device must have a persistable id"
            );
            assert!(
                !d.name.is_empty(),
                "a listed device must have a display name"
            );
            assert!(!d.backend.is_empty());
        }
        let defaults = devices.iter().filter(|d| d.is_default).count();
        assert!(
            defaults <= 1,
            "at most one device can be the system default"
        );
    }

    #[test]
    fn the_system_default_resolves_and_reports_what_was_opened() {
        match resolve(&DeviceRequest::SystemDefault) {
            Ok((_, descriptor)) => {
                assert!(!descriptor.id.is_empty());
                assert!(!descriptor.name.is_empty());
            }
            Err(DeviceError::NoOutputDevice) => {
                eprintln!("skipping: machine has no output device");
            }
            Err(e) => panic!("unexpected backend failure: {e}"),
        }
    }

    #[test]
    fn a_missing_specific_device_falls_back_to_the_default_rather_than_failing() {
        let request = DeviceRequest::Specific("definitely-not-a-real-device-id".into());
        match resolve(&request) {
            Ok((_, descriptor)) => {
                assert_ne!(
                    descriptor.id, "definitely-not-a-real-device-id",
                    "must not claim to have opened a device that does not exist"
                );
                assert!(
                    !descriptor.id.is_empty(),
                    "should report the device actually opened"
                );
            }
            Err(DeviceError::NoOutputDevice) => {
                eprintln!("skipping: machine has no output device");
            }
            Err(e) => panic!("unexpected backend failure: {e}"),
        }
    }
}
