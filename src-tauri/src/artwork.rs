//! Track artwork bytes for the frontend's color extraction (`src/lib/artworkColors.ts`).
//!
//! The webview can display remote artwork in an `<img>`, but it cannot read the pixels
//! back through a canvas unless the image host sends CORS headers, which artwork hosts
//! don't promise. Fetching here, through the shared artwork profile (finite deadline, byte
//! cap), hands the frontend bytes it can decode as a same-origin blob.

use crate::network::{redact_text, NetworkService};
use tauri::{ipc::Response, State};
use tokio_util::sync::CancellationToken;

#[tauri::command]
pub async fn artwork_fetch(
    network: State<'_, NetworkService>,
    url: String,
) -> Result<Response, String> {
    let cancel = CancellationToken::new();
    let body = network
        .fetch_artwork(&url, &cancel)
        .await
        .map_err(|error| redact_text(&error.to_string()))?;
    if !is_image(body.content_type.as_deref()) {
        return Err("artwork response was not an image".into());
    }
    if body.bytes.is_empty() {
        return Err("artwork response was empty".into());
    }
    Ok(Response::new(body.bytes))
}

/// A missing content type is let through; the webview's image decoder is the final check.
fn is_image(content_type: Option<&str>) -> bool {
    content_type.is_none_or(|value| value.trim().to_ascii_lowercase().starts_with("image/"))
}

#[cfg(test)]
mod tests {
    use super::is_image;

    #[test]
    fn accepts_image_types_and_unlabelled_bodies() {
        assert!(is_image(Some("image/jpeg")));
        assert!(is_image(Some("Image/PNG; charset=binary")));
        assert!(is_image(None));
    }

    #[test]
    fn rejects_non_image_types() {
        assert!(!is_image(Some("text/html")));
        assert!(!is_image(Some("application/json")));
    }
}
