fn main() {
    // Local dev convenience only - CI already has STELLAR_API_KEY in the process
    // env via the repo's Actions secret, so this is a no-op there.
    if std::env::var("STELLAR_API_KEY").is_err() || std::env::var("LASTFM_SHARED_SECRET").is_err() {
        let _ = dotenvy::from_filename(".env");
    }

    // Do not build with -vv locally - build script stdout (including this line)
    // can end up in verbose cargo/CI logs. Only emit the rustc-env line when a
    // key is actually present, so option_env!("STELLAR_API_KEY") in secrets.rs
    // resolves to None at compile time for builds without one (e.g. unconfigured
    // local dev), rather than baking in an empty-string sentinel.
    if let Ok(key) = std::env::var("STELLAR_API_KEY") {
        println!("cargo:rustc-env=STELLAR_API_KEY={key}");
    }
    println!("cargo:rerun-if-env-changed=STELLAR_API_KEY");
    if let Ok(secret) = std::env::var("LASTFM_SHARED_SECRET") {
        println!("cargo:rustc-env=LASTFM_SHARED_SECRET={secret}");
    }
    println!("cargo:rerun-if-env-changed=LASTFM_SHARED_SECRET");
    println!("cargo:rerun-if-changed=.env");

    tauri_build::build()
}
