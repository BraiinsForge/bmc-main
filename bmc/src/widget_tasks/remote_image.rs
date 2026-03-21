// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::{Context, Result, bail};
use bmc_display::data::{ImageScaleMode, SceneId, WidgetId, WidgetSize};
use bmc_display::display_controller::DisplayController;
use bmc_display::remote_image_data::RemoteImageState;
use bmc_display::{SharedImageBuffer, SharedPixelBuffer};
use image::GenericImageView;
use reqwest::Client;
use std::io;
use std::time::Duration;
use tokio::time::{Instant, MissedTickBehavior, interval};
use tracing::{Instrument, debug, info, instrument, warn};
use url::Url;

const INITIAL_LOADING_DELAY: Duration = Duration::from_millis(300);
const MAX_SOURCE_DIMENSION: u32 = 4096;
/// 4096 × 4096 × 4 (RGBA) = 64 MiB decoded; raw body should be well under this.
const MAX_BODY_SIZE: u64 = 64 * 1024 * 1024;

#[instrument(name = "remote_image", skip_all, fields(%scene_id, %widget_id))]
pub async fn run(
    display_controller: DisplayController,
    scene_id: SceneId,
    widget_id: WidgetId,
    widget_size: WidgetSize,
    url: String,
    refresh_duration: Duration,
    image_scale_mode: ImageScaleMode,
) {
    let widget_width = widget_size.width();
    let widget_height = widget_size.height();

    info!(
        ?widget_size,
        widget_width,
        widget_height,
        url,
        ?refresh_duration,
        ?image_scale_mode,
        "Params"
    );

    let url = url
        .replace("{{width}}", &widget_width.to_string())
        .replace("{{height}}", &widget_height.to_string());

    let parsed_url = match Url::parse(&url) {
        Ok(url) => url,
        Err(err) => {
            warn!(?err, "Invalid URL, stopping");
            display_controller.update_remote_image(
                scene_id.clone(),
                widget_id.clone(),
                RemoteImageState::ConfigurationError,
                String::new(),
            );
            return;
        }
    };

    let client = match Client::builder()
        .timeout(Duration::from_secs(120))
        // NOTE: we don't care, since we are not sending any sensitive data
        .danger_accept_invalid_certs(true)
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            warn!(?err, "Failed to create reqwest client, stopping");
            display_controller.update_remote_image(
                scene_id.clone(),
                widget_id.clone(),
                RemoteImageState::UnexpectedError,
                String::new(),
            );
            return;
        }
    };

    // NOTE: intentionally initialized here, not at the beginning of the async block.
    // This way it will be dropped only when task is aborted.
    let _drop_guard = ResetToInitialStateDropGuard {
        scene_id: scene_id.clone(),
        widget_id: widget_id.clone(),
        display_controller: display_controller.clone(),
    };

    let mut decoder_limits = image::Limits::no_limits();
    decoder_limits.max_image_width = Some(MAX_SOURCE_DIMENSION);
    decoder_limits.max_image_height = Some(MAX_SOURCE_DIMENSION);
    // Non-strict: best-effort cap on decoder allocations (64 MiB).
    // Strict dimension limits above are the primary safeguard.
    decoder_limits.max_alloc = Some(MAX_BODY_SIZE);

    let mut interval = interval(refresh_duration);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let mut last_success: Option<Instant> = None;
    let mut prev_failed = false;

    // Show "Waiting for initial data" after INITIAL_LOADING_DELAY if the
    // first load hasn't completed yet.
    let mut loading_task = Some(tokio::spawn({
        let dc = display_controller.clone();
        let sid = scene_id.clone();
        let wid = widget_id.clone();
        async move {
            tokio::time::sleep(INITIAL_LOADING_DELAY).await;
            dc.update_remote_image(sid, wid, RemoteImageState::Loading, String::new());
        }
    }));

    loop {
        interval.tick().await;

        let state = load_image(
            &client,
            &parsed_url,
            &decoder_limits,
            widget_width,
            widget_height,
            image_scale_mode,
        )
        .in_current_span()
        .await
        .map_or_else(
            RemoteImageState::LoadingError,
            RemoteImageState::LoadingSuccess,
        );

        if let RemoteImageState::LoadingError(err) = &state {
            warn!(?err);

            let retry_secs = refresh_duration.as_secs().clamp(10, 60);
            interval.reset_after(Duration::from_secs(retry_secs));

            let stale_text = if prev_failed {
                last_success
                    .map(super::format_stale_text)
                    .unwrap_or_default()
            } else {
                String::new()
            };
            prev_failed = true;

            display_controller.update_remote_image(
                scene_id.clone(),
                widget_id.clone(),
                state,
                stale_text,
            );
        } else {
            if let Some(task) = loading_task.take() {
                task.abort();
            }

            last_success = Some(Instant::now());
            prev_failed = false;

            display_controller.update_remote_image(
                scene_id.clone(),
                widget_id.clone(),
                state,
                String::new(),
            );
        }
    }
}

async fn load_image(
    client: &Client,
    parsed_url: &Url,
    decoder_limits: &image::Limits,
    widget_width: u32,
    widget_height: u32,
    image_scale_mode: ImageScaleMode,
) -> Result<SharedImageBuffer> {
    info!("Sending request to get image");

    let start = Instant::now();

    let mut response = client
        .get(parsed_url.clone())
        .send()
        .await
        .context("Failed to get image")?
        .error_for_status()
        .context("Server returned an error")?;

    if let Some(len) = response.content_length()
        && len > MAX_BODY_SIZE
    {
        bail!("Image too large: {len} bytes (max {MAX_BODY_SIZE})");
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("Failed to read chunk from the response")?
    {
        bytes.extend_from_slice(&chunk);
        if bytes.len() as u64 > MAX_BODY_SIZE {
            bail!(
                "Image body exceeded limit while streaming: {} bytes (max {MAX_BODY_SIZE})",
                bytes.len()
            );
        }
    }

    info!(duration = ?start.elapsed(), "Response received successfully");

    let mut reader = image::ImageReader::new(io::Cursor::new(bytes));
    reader.limits(decoder_limits.clone());

    let decoded = reader
        .with_guessed_format()
        .expect("BUG: Seek for io::Cursor cannot fail")
        .decode()
        .context("Failed to decode image")?;

    let (img_w, img_h) = decoded.dimensions();

    let fitted = if img_w == widget_width && img_h == widget_height {
        decoded.into_rgba8()
    } else {
        debug!(
            img_w,
            img_h,
            widget_width,
            widget_height,
            ?image_scale_mode,
            "Scaling image"
        );
        match image_scale_mode {
            ImageScaleMode::Fill => decoded
                .resize_to_fill(
                    widget_width,
                    widget_height,
                    image::imageops::FilterType::Triangle,
                )
                .into_rgba8(),
            ImageScaleMode::Fit => {
                let resized = decoded
                    .resize(
                        widget_width,
                        widget_height,
                        image::imageops::FilterType::Triangle,
                    )
                    .into_rgba8();
                let mut canvas = image::RgbaImage::from_pixel(
                    widget_width,
                    widget_height,
                    image::Rgba([0, 0, 0, 255]),
                );
                #[expect(clippy::integer_division)]
                let offset_x = (widget_width - resized.width()) / 2;
                #[expect(clippy::integer_division)]
                let offset_y = (widget_height - resized.height()) / 2;
                image::imageops::overlay(
                    &mut canvas,
                    &resized,
                    i64::from(offset_x),
                    i64::from(offset_y),
                );
                canvas
            }
        }
    };

    let mut buffer = SharedPixelBuffer::new(widget_width, widget_height);
    buffer.make_mut_bytes().copy_from_slice(fitted.as_raw());
    Ok(SharedImageBuffer::RGBA8(buffer))
}

struct ResetToInitialStateDropGuard {
    scene_id: SceneId,
    widget_id: WidgetId,
    display_controller: DisplayController,
}

impl Drop for ResetToInitialStateDropGuard {
    fn drop(&mut self) {
        self.display_controller.update_remote_image(
            self.scene_id.clone(),
            self.widget_id.clone(),
            RemoteImageState::Initial,
            String::new(),
        );
    }
}
