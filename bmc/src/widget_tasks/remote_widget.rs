// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::config::{ConfigHandle, TemperatureUnit};
use anyhow::{Context, Result, bail};
use backon::{BackoffBuilder, ExponentialBuilder};
use bmc_display::data::{SceneId, WidgetId, WidgetSize};
use bmc_display::display_controller::DisplayController;
use bmc_display::remote_widget_data::RemoteWidgetState;
use bmc_display::{SharedImageBuffer, SharedPixelBuffer};
use bmc_shared_time::time::{DateFormat, TimeSystem, Timezone};
use bmc_shared_utils::number_format::NumberFormat;
use image::ImageDecoder;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, watch};
use tokio::time::{MissedTickBehavior, interval};
use tracing::{Instrument, info, instrument, warn};
use url::Url;

const INIT_REFRESH_PERIOD: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Serialize)]
struct InvokeBody {
    size: String,
    widget_version: String,
    params: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct InvokeResponse {
    pub image_url: String,
    pub next_trigger_secs: u64,
}

#[instrument(name = "remote_widget", skip_all, fields(%scene_id, %widget_id))]
pub async fn run(
    display_controller: DisplayController,
    scene_id: SceneId,
    widget_id: WidgetId,
    widget_size: WidgetSize,
    config_handle: Arc<RwLock<ConfigHandle>>,
    system_timezone_receiver: watch::Receiver<Timezone>,
    url: String,
) {
    let error_backoff_builder = ExponentialBuilder::new()
        .with_min_delay(Duration::from_secs(10))
        .with_max_delay(Duration::from_secs(5 * 60))
        .with_factor(2.0)
        .without_max_times();

    let mut error_backoff = error_backoff_builder.build();

    info!(?widget_size, url, ?error_backoff, "Params");

    let base_url = match Url::parse(&url) {
        Ok(url) => url,
        Err(err) => {
            warn!(?err, "Invalid URL, stopping");
            display_controller.update_remote_widget(
                scene_id.clone(),
                widget_id.clone(),
                RemoteWidgetState::ConfigurationError,
            );
            return;
        }
    };

    let mut invoke_url = base_url.clone();
    match invoke_url.path_segments_mut() {
        Ok(mut path_segments) => path_segments.push("invoke"),
        Err(err) => {
            warn!(?err, "Failed to add path segments");
            display_controller.update_remote_widget(
                scene_id.clone(),
                widget_id.clone(),
                RemoteWidgetState::UnexpectedError,
            );
            return;
        }
    };

    let client = match Client::builder().timeout(REQUEST_TIMEOUT).build() {
        Ok(client) => client,
        Err(err) => {
            warn!(?err, "Failed to create reqwest client, stopping");
            display_controller.update_remote_widget(
                scene_id.clone(),
                widget_id.clone(),
                RemoteWidgetState::UnexpectedError,
            );
            return;
        }
    };

    let params = {
        let config = config_handle.read().await;
        let user_params = match config
            .scenes
            .get(&scene_id)
            .and_then(|s| s.widgets.get(&widget_id))
            .and_then(|w| {
                if let bmc_display::data::WidgetKind::RemoteWidget(remote_widget) = &w.kind {
                    Some(remote_widget)
                } else {
                    display_controller.update_remote_widget(
                        scene_id.clone(),
                        widget_id.clone(),
                        RemoteWidgetState::UnexpectedError,
                    );
                    None
                }
            }) {
            Some(remote_widget) if remote_widget.params != serde_json::Value::Null => {
                remote_widget.params.clone()
            }
            Some(_) => serde_json::json!({}),
            None => return,
        };

        let localization = config.localization_config();
        let timezone = system_timezone_receiver.borrow();

        // TODO: Ideally, this JSON schema should be generated from protobufs
        // rather than being hardcoded here.
        // Start with system prefs as base
        let mut params = serde_json::json!({
            "timezone": timezone.iana(),
            "numberFormat": format_number_format(localization.number_format),
            "dateFormat": format_date_format(localization.date_format),
            "timeFormat": format_time_format(localization.time_system),
            "temperatureUnit": format_temperature_unit(&localization.temperature_unit)
        });

        // Merge user params on top (user params take precedence)
        if let (Some(base), Some(user)) = (params.as_object_mut(), user_params.as_object()) {
            base.extend(user.iter().map(|(k, v)| (k.clone(), v.clone())));
        }

        params
    };

    let invoke_body = InvokeBody {
        size: widget_size.to_string(),
        widget_version: String::new(),
        params,
    };

    // info!(?invoke_body, "Remote widget invoke body"); for debugging

    // NOTE: intentionally initialized here, not at the beginning of the async block.
    // This way it will be dropped only when task is aborted.
    let _drop_guard = ResetToInitialStateDropGuard {
        scene_id: scene_id.clone(),
        widget_id: widget_id.clone(),
        display_controller: display_controller.clone(),
    };

    let mut interval = interval(INIT_REFRESH_PERIOD);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    display_controller.update_remote_widget(
        scene_id.clone(),
        widget_id.clone(),
        RemoteWidgetState::Loading,
    );

    loop {
        interval.tick().await;

        match invoke_method(&client, &invoke_url, &invoke_body).await {
            Ok(invoke_response) => {
                let state = get_widget_image(&client, &base_url, invoke_response.image_url)
                    .in_current_span()
                    .await
                    .map_or_else(
                        RemoteWidgetState::LoadingError,
                        RemoteWidgetState::LoadingSuccess,
                    );

                if let RemoteWidgetState::LoadingError(err) = &state {
                    warn!(?err);

                    if let Some(duration) = error_backoff.next() {
                        interval.reset_after(duration);
                    }
                } else {
                    // NOTE: backoff does not have `reset` method, so we need to recreate it
                    error_backoff = error_backoff_builder.build();
                    interval.reset_after(Duration::from_secs(invoke_response.next_trigger_secs));
                }

                display_controller.update_remote_widget(scene_id.clone(), widget_id.clone(), state);
            }
            Err(err) => {
                warn!(?err);
                if let Some(duration) = error_backoff.next() {
                    interval.reset_after(duration);
                }
                display_controller.update_remote_widget(
                    scene_id.clone(),
                    widget_id.clone(),
                    RemoteWidgetState::LoadingError(err),
                );
            }
        }
    }
}

async fn invoke_method(
    client: &Client,
    invoke_url: &Url,
    invoke_body: &InvokeBody,
) -> Result<InvokeResponse> {
    client
        .post(invoke_url.clone())
        .json(&invoke_body)
        .send()
        .await
        .context("Failed to get invoke method response")?
        .error_for_status()
        .context("Server returned an error")?
        .json::<InvokeResponse>()
        .await
        .context("Failed to parse the invoke method response")
}

async fn get_widget_image(
    client: &Client,
    base_url: &Url,
    image_url: String,
) -> Result<SharedImageBuffer> {
    let widget_image_url = base_url
        .join(&image_url)
        .context("Failed to add path segments")?;

    let image_bytes = client
        .get(widget_image_url)
        .send()
        .await
        .context("Failed to get widget image")?
        .error_for_status()
        .context("Server returned an error")?
        .bytes()
        .await
        .context("Failed to read bytes from the response")?;

    let mut reader = image::ImageReader::new(io::Cursor::new(image_bytes));
    reader.set_format(image::ImageFormat::Png);

    let decoder = reader
        .with_guessed_format()
        .expect("BUG: Seek for io::Cursor cannot fail")
        .into_decoder()
        .context("Failed to initialize image decoder")?;

    let (width, height) = decoder.dimensions();

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

struct ResetToInitialStateDropGuard {
    scene_id: SceneId,
    widget_id: WidgetId,
    display_controller: DisplayController,
}

impl Drop for ResetToInitialStateDropGuard {
    fn drop(&mut self) {
        self.display_controller.update_remote_widget(
            self.scene_id.clone(),
            self.widget_id.clone(),
            RemoteWidgetState::Initial,
        );
    }
}

fn format_number_format(value: NumberFormat) -> &'static str {
    match value {
        NumberFormat::SpaceGroupCommaDecimal => "NUMBER_FORMAT_SPACE_GROUP_COMMA_DECIMAL",
        NumberFormat::CommaGroupDotDecimal => "NUMBER_FORMAT_COMMA_GROUP_DOT_DECIMAL",
        NumberFormat::DotGroupCommaDecimal => "NUMBER_FORMAT_DOT_GROUP_COMMA_DECIMAL",
        NumberFormat::SpaceGroupDotDecimal => "NUMBER_FORMAT_SPACE_GROUP_DOT_DECIMAL",
    }
}

fn format_time_format(value: TimeSystem) -> &'static str {
    match value {
        TimeSystem::Hour12 => "TIME_FORMAT_12_HOUR",
        TimeSystem::Hour24 => "TIME_FORMAT_24_HOUR",
    }
}

fn format_date_format(value: DateFormat) -> &'static str {
    match value {
        DateFormat::DdMmYyyyDot => "DATE_FORMAT_DD_MM_YYYY_DOT",
        DateFormat::DdMmYyyySlash => "DATE_FORMAT_DD_MM_YYYY_SLASH",
        DateFormat::DMYyyySlash => "DATE_FORMAT_D_M_YYYY_SLASH",
        DateFormat::MDYyyySlash => "DATE_FORMAT_M_D_YYYY_SLASH",
        DateFormat::DdMmYyyyDash => "DATE_FORMAT_DD_MM_YYYY_DASH",
        DateFormat::YyyyMDSlash => "DATE_FORMAT_YYYY_M_D_SLASH",
        DateFormat::YyyyMmDdDot => "DATE_FORMAT_YYYY_MM_DD_DOT",
        DateFormat::YyyyMmDdDash => "DATE_FORMAT_YYYY_MM_DD_DASH",
    }
}

fn format_temperature_unit(value: &TemperatureUnit) -> &'static str {
    match value {
        TemperatureUnit::Celsius => "TEMPERATURE_UNIT_CELSIUS",
        TemperatureUnit::Fahrenheit => "TEMPERATURE_UNIT_FAHRENHEIT",
    }
}
