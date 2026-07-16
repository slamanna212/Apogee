use keyring::{Entry, Error};

const SERVICE: &str = "com.slamanna.apogee";

fn entry(key: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, key).map_err(|e| e.to_string())
}

pub(crate) fn set(key: &str, value: &str) -> Result<(), String> {
    entry(key)?.set_password(value).map_err(|e| e.to_string())
}

pub(crate) fn get(key: &str) -> Result<Option<String>, String> {
    match entry(key)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub(crate) fn delete(key: &str) -> Result<(), String> {
    match entry(key)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Keys the frontend is allowed to touch through the generic secrets_*
/// commands below. Other keyring entries under this app's service name
/// (e.g. the Last.fm session key/username, set via `secrets::set` directly
/// from lastfm.rs) are never routed through a generic command reachable
/// from JS, and shouldn't become readable/overwritable through this one
/// just because a caller happens to know the key string.
const ALLOWED_FRONTEND_KEYS: &[&str] = &["xtream_password"];

fn check_allowed_key(key: &str) -> Result<(), String> {
    if ALLOWED_FRONTEND_KEYS.contains(&key) {
        Ok(())
    } else {
        Err(format!("secrets: key \"{key}\" is not allowed"))
    }
}

#[tauri::command]
pub fn secrets_set(key: String, value: String) -> Result<(), String> {
    check_allowed_key(&key)?;
    set(&key, &value)
}

#[tauri::command]
pub fn secrets_get(key: String) -> Result<Option<String>, String> {
    check_allowed_key(&key)?;
    get(&key)
}

#[tauri::command]
pub fn secrets_delete(key: String) -> Result<(), String> {
    check_allowed_key(&key)?;
    delete(&key)
}

/// Baked in at compile time by build.rs from the STELLAR_API_KEY env var
/// (the repo's Actions secret in CI, or a git-ignored src-tauri/.env locally).
/// Resolves to None when no key was available at build time.
#[tauri::command]
pub fn secrets_get_builtin_stellar_key() -> Option<String> {
    option_env!("STELLAR_API_KEY").map(str::to_string)
}
