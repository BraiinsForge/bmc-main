// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::{Context, bail};
use backon::{BackoffBuilder, ExponentialBuilder};
use bmc_display::data::{SceneId, WidgetId, WidgetSize};
use bmc_display::display_controller::DisplayController;
use bmc_display::remote_image_data::RemoteImageState;
use bmc_display::{SharedImageBuffer, SharedPixelBuffer};
use image::ImageDecoder;
use reqwest::Client;
use std::io;
use std::time::Duration;
use tokio::time::{Instant, MissedTickBehavior, interval};
use tracing::{Instrument, info, instrument, warn};
use url::Url;

#[instrument(name = "remote_image", skip_all, fields(%scene_id, %widget_id))]
pub async fn run(
    display_controller: DisplayController,
    scene_id: SceneId,
    widget_id: WidgetId,
    widget_size: WidgetSize,
    url: String,
    refresh_duration: Duration,
) {
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
            );
        }
    }

    let error_backoff_builder = ExponentialBuilder::new()
        .with_min_delay(Duration::from_secs(10))
        .with_max_delay(Duration::from_secs(60 * 5).min(refresh_duration))
        .with_factor(2.0)
        .without_max_times();

    let mut error_backoff = error_backoff_builder.build();

    let widget_width = widget_size.width();
    let widget_height = widget_size.height();

    info!(
        ?widget_size,
        widget_width,
        widget_height,
        url,
        ?refresh_duration,
        ?error_backoff,
        "Params"
    );

    let mut parsed_url = match Url::parse(&url) {
        Ok(url) => url,
        Err(err) => {
            warn!(?err, "Invalid URL, stopping");
            display_controller.update_remote_image(
                scene_id.clone(),
                widget_id.clone(),
                RemoteImageState::ConfigurationError,
            );
            return;
        }
    };

    // NOTE: we provide dimensions in query params for advanced users.
    // They can implement single endpoint, which can dynamically generate image.
    // `deck_image_` prefix is here to prevent collisions with query params provided by the user.
    parsed_url
        .query_pairs_mut()
        .append_pair("deck_image_width", &widget_width.to_string())
        .append_pair("deck_image_height", &widget_height.to_string());

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
    decoder_limits.max_image_width = Some(widget_width);
    decoder_limits.max_image_height = Some(widget_height);

    let mut interval = interval(refresh_duration);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    display_controller.update_remote_image(
        scene_id.clone(),
        widget_id.clone(),
        RemoteImageState::Loading,
    );

    loop {
        // NOTE: this might take `refresh_duration` or `error_refresh_duration` time
        interval.tick().await;

        let state = async {
            info!("Sending request to get remote image");

            let start = Instant::now();

            let bytes = client
                .get(parsed_url.clone())
                .send()
                .await
                .context("Failed to get remote image")?
                .error_for_status()
                .context("Server returned an error")?
                .bytes()
                .await
                .context("Failed to read bytes from the response")?;

            info!(duration = ?start.elapsed(), "Response received successfully");

            let mut reader = image::ImageReader::new(io::Cursor::new(bytes));
            reader.limits(decoder_limits.clone());
            reader.set_format(image::ImageFormat::Png);

            let decoder = reader
                .with_guessed_format()
                .expect("BUG: Seek for io::Cursor cannot fail")
                .into_decoder()
                .context("Failed to initialize image decoder")?;

            let (width, height) = decoder.dimensions();

            if width != widget_width || height != widget_height {
                warn!(
                    width,
                    height,
                    expected_width = widget_width,
                    expected_height = widget_height,
                    "Unexpected image dimensions"
                );
                bail!("Unexpected image dimensions");
            }

            #[expect(clippy::wildcard_enum_match_arm)]
            match decoder.color_type() {
                image::ColorType::Rgb8 => {
                    let mut buffer = SharedPixelBuffer::new(width, height);

                    decoder
                        .read_image(buffer.make_mut_bytes())
                        .map(|()| SharedImageBuffer::RGB8(buffer))
                }
                image::ColorType::Rgba8 => {
                    let mut buffer = SharedPixelBuffer::new(width, height);

                    decoder
                        .read_image(buffer.make_mut_bytes())
                        .map(|()| SharedImageBuffer::RGBA8(buffer))
                }
                color_type => {
                    bail!("Unexpected color type: {color_type:?}");
                }
            }
            .context("Failed to decode image")
        }
        .in_current_span()
        .await
        .map_or_else(
            RemoteImageState::LoadingError,
            RemoteImageState::LoadingSuccess,
        );

        if let RemoteImageState::LoadingError(err) = &state {
            warn!(?err);

            if let Some(duration) = error_backoff.next() {
                interval.reset_after(duration);
            }
        } else {
            // NOTE: backoff does not have `reset` method, so we need to recreate it
            error_backoff = error_backoff_builder.build();
        }

        display_controller.update_remote_image(scene_id.clone(), widget_id.clone(), state);
    }
}
