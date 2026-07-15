use keyring::{Entry, Error};

const SERVICE: &str = "com.slamanna.apogee";

fn entry(key: &str) -> Result<Entry, String> {
  Entry::new(SERVICE, key).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn secrets_set(key: String, value: String) -> Result<(), String> {
  entry(&key)?.set_password(&value).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn secrets_get(key: String) -> Result<Option<String>, String> {
  match entry(&key)?.get_password() {
    Ok(value) => Ok(Some(value)),
    Err(Error::NoEntry) => Ok(None),
    Err(e) => Err(e.to_string()),
  }
}

#[tauri::command]
pub fn secrets_delete(key: String) -> Result<(), String> {
  match entry(&key)?.delete_credential() {
    Ok(()) | Err(Error::NoEntry) => Ok(()),
    Err(e) => Err(e.to_string()),
  }
}

/// Baked in at compile time by build.rs from the STELLAR_API_KEY env var
/// (the repo's Actions secret in CI, or a git-ignored src-tauri/.env locally).
/// Resolves to None when no key was available at build time.
#[tauri::command]
pub fn secrets_get_builtin_stellar_key() -> Option<String> {
  option_env!("STELLAR_API_KEY").map(str::to_string)
}
