use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::{ApiError, Services, bad, failed};

#[derive(Serialize)]
pub(super) struct DisplayStatus {
    modes: Vec<crate::output::display::ModeInfo>,
    current: Option<crate::output::display::ModeInfo>,
    outputs: Vec<crate::output::display::OutputInfo>,
    /// Why modes couldn't be listed, e.g. nothing plugged in
    error: Option<String>,
}

impl DisplayStatus {
    fn unavailable(error: String) -> Self {
        Self {
            modes: crate::output::display::available_modes(Vec::new()),
            current: None,
            outputs: Vec::new(),
            error: Some(error),
        }
    }
}

pub(super) async fn display_status(State(sv): State<Services>) -> Json<DisplayStatus> {
    let player = Arc::clone(&sv.player);
    let result = tokio::task::spawn_blocking(move || player.modes())
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    Json(match result {
        Ok((modes, current, outputs)) => DisplayStatus {
            modes: crate::output::display::available_modes(modes),
            current,
            outputs,
            error: None,
        },
        Err(e) => DisplayStatus::unavailable(e),
    })
}

#[derive(Serialize)]
pub(super) struct SplashStatus {
    custom_image: bool,
    custom_video: bool,
}

#[derive(Serialize)]
pub(super) struct OverlayStatus {
    /// Pixel size of the uploaded logo, None without one
    image: Option<(u32, u32)>,
    version: u64,
}

pub(super) async fn overlay_status(State(sv): State<Services>) -> Json<OverlayStatus> {
    Json(OverlayStatus {
        image: sv.overlay.logo().map(|l| (l.width, l.height)),
        version: sv.overlay.version(),
    })
}

/// Redraw wherever the logo shows. A USB camera has it encoded in, so its
/// pipeline restarts
pub(super) fn overlay_changed(sv: &Services) {
    if sv.input.mode() == crate::settings::InputMode::Uvc {
        sv.uvc.apply(&sv.store.get().input.uvc);
    }
    sv.player.refresh_logo();
}

pub(super) async fn overlay_upload(
    State(sv): State<Services>,
    image: axum::body::Bytes,
) -> Result<StatusCode, ApiError> {
    let overlay = Arc::clone(&sv.overlay);
    tokio::task::spawn_blocking(move || overlay.set_image(&image))
        .await
        .map_err(|e| failed(e.to_string()))?
        .map_err(|e| bad(format!("Couldn't use that image: {e}")))?;
    overlay_changed(&sv);
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn overlay_remove(State(sv): State<Services>) -> Result<StatusCode, ApiError> {
    sv.overlay.remove().map_err(failed)?;
    overlay_changed(&sv);
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn splash_status(State(sv): State<Services>) -> Json<SplashStatus> {
    Json(SplashStatus {
        custom_image: sv.splash.has_custom_image(),
        custom_video: sv.splash.has_custom_video(),
    })
}

#[derive(Deserialize)]
pub(super) struct PreviewQuery {
    status_over_image: Option<bool>,
    image_scaling: Option<crate::settings::Scaling>,
}

pub(super) async fn splash_preview(
    State(sv): State<Services>,
    Query(query): Query<PreviewQuery>,
) -> Result<Response, ApiError> {
    use axum::http::header;
    let (image, mime) = tokio::task::spawn_blocking(move || {
        sv.splash
            .preview(query.status_over_image, query.image_scaling)
    })
    .await
    .map_err(|e| failed(e.to_string()))?
    .map_err(|e| ApiError(StatusCode::SERVICE_UNAVAILABLE, e.to_string()))?;
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "no-store"),
        ],
        image,
    )
        .into_response())
}

pub(super) async fn splash_upload(
    State(sv): State<Services>,
    image: axum::body::Bytes,
) -> Result<StatusCode, ApiError> {
    tokio::task::spawn_blocking(move || sv.splash.set_custom_image(&image))
        .await
        .map_err(|e| failed(e.to_string()))?
        .map_err(|e| bad(format!("Couldn't use that image: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn splash_remove(State(sv): State<Services>) -> Result<StatusCode, ApiError> {
    sv.splash
        .remove_custom_image()
        .map_err(|e| failed(format!("Couldn't remove the image: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn splash_video_upload(
    State(sv): State<Services>,
    body: axum::body::Body,
) -> Result<StatusCode, ApiError> {
    use crate::output::splash::video::MAX_UPLOAD_BYTES;
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    // Reserve the slot before receiving the body. Concurrent uploads cannot
    // consume another file's worth of disk or overwrite an ongoing conversion.
    let upload = sv
        .splash
        .begin_video_upload()
        .map_err(|error| ApiError(StatusCode::CONFLICT, error.to_string()))?;
    let mut file = tokio::fs::File::create(upload.source())
        .await
        .map_err(|e| failed(e.to_string()))?;
    let mut stream = body.into_data_stream();
    let receive = async {
        let mut received = 0usize;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| bad(format!("Video upload interrupted: {e}")))?;
            received = received.checked_add(chunk.len()).ok_or_else(|| {
                ApiError(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "Video upload exceeds 256 MiB".into(),
                )
            })?;
            if received > MAX_UPLOAD_BYTES {
                return Err(ApiError(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "Video upload exceeds 256 MiB".into(),
                ));
            }
            file.write_all(&chunk)
                .await
                .map_err(|e| failed(e.to_string()))?;
        }
        if received == 0 {
            return Err(bad("Choose a video file to upload"));
        }
        file.flush().await.map_err(|e| failed(e.to_string()))?;
        Ok(())
    };
    tokio::time::timeout(std::time::Duration::from_secs(10 * 60), receive)
        .await
        .map_err(|_| ApiError(StatusCode::REQUEST_TIMEOUT, "Video upload timed out".into()))??;
    drop(file);
    sv.splash
        .finish_video_upload(upload)
        .await
        .map_err(|e| bad(format!("Couldn't use that video: {e:#}")))?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn splash_video_remove(
    State(sv): State<Services>,
) -> Result<StatusCode, ApiError> {
    sv.splash
        .remove_custom_video()
        .map_err(|e| bad(format!("Couldn't remove the video: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn a_disconnected_display_still_returns_the_manual_catalogue() {
        let status = DisplayStatus::unavailable("Waiting for a screen".into());
        assert_eq!(status.modes.len(), 31);
        assert!(status.modes.iter().any(|mode| mode.id == "1920x1080@60.00"));
        assert!(
            status
                .modes
                .iter()
                .any(|mode| mode.id == "1920x1080i@60.00")
        );
        assert_eq!(status.error.as_deref(), Some("Waiting for a screen"));
        assert!(status.current.is_none() && status.outputs.is_empty());
    }
}
