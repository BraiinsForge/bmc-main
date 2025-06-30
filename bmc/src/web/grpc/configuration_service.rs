// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::system_manager::SystemManager;

use bmc_display::display_driver::DisplayBacklightDriver;
use bmc_grpc::web::{
    BrightnessInfo, DisplaySettingsResponse, TimeInterval,
    configuration_service_server::ConfigurationService as GrpcConfigurationService,
};
use chrono::NaiveTime;
use tonic::{Request, Response, Status};
use tracing::{error, warn};

const API_BRIGHTNESS_MIN: u32 = 0;
const API_BRIGHTNESS_MAX: u32 = 100;
const API_BRIGHTNESS_STEP: u32 = 5;

pub(crate) struct ConfigurationService<T: DisplayBacklightDriver> {
    system_manager: SystemManager<T>,
}

impl<T: DisplayBacklightDriver> ConfigurationService<T> {
    pub(crate) fn new(system_manager: SystemManager<T>) -> Self {
        Self { system_manager }
    }
}

#[async_trait::async_trait]

impl<T: DisplayBacklightDriver> GrpcConfigurationService for ConfigurationService<T> {
    async fn get_display_settings(
        &self,
        _request: Request<()>,
    ) -> Result<Response<DisplaySettingsResponse>, Status> {
        let display_settings = self.system_manager.display_settings().await;

        Ok(Response::new(DisplaySettingsResponse {
            brightness: Some(BrightnessInfo {
                value: u32::from(display_settings.brightness_pct),
                min: API_BRIGHTNESS_MIN,
                max: API_BRIGHTNESS_MAX,
                step: API_BRIGHTNESS_STEP,
            }),

            brightness_nightmode: Some(BrightnessInfo {
                value: u32::from(display_settings.night_mode_config.brightness_pct),
                min: API_BRIGHTNESS_MIN,
                max: API_BRIGHTNESS_MAX,
                step: API_BRIGHTNESS_STEP,
            }),
            nightmode_enabled: display_settings.night_mode_config.enabled,
            nightmode_interval: Some(TimeInterval {
                from: naive_time_to_hhmm(display_settings.night_mode_config.from),
                to: naive_time_to_hhmm(display_settings.night_mode_config.to),
            }),
        }))
    }

    async fn set_brightness(&self, request: Request<u32>) -> Result<Response<()>, Status> {
        let value = request.into_inner();

        validate_brightness(value)?;

        #[expect(clippy::cast_possible_truncation)]
        self.system_manager
            .set_brightness(value as u8)
            .await
            .map_err(|e| {
                warn!("Cannot set display brightness: {}", e);
                Status::internal("Failed to set display brightness")
            })?;

        Ok(Response::new(()))
    }

    async fn set_brightness_nightmode(
        &self,
        request: Request<u32>,
    ) -> Result<Response<()>, Status> {
        let value = request.into_inner();

        validate_brightness(value)?;
        #[expect(clippy::cast_possible_truncation)]
        self.system_manager
            .set_night_mode_brightness(value as u8)
            .await
            .map_err(|e| {
                warn!("Cannot set night mode brightness, error: {}", e);
                Status::internal("Failed to set night mode brightness")
            })?;

        Ok(Response::new(()))
    }

    async fn set_nightmode_enabled(&self, request: Request<bool>) -> Result<Response<()>, Status> {
        let value = request.into_inner();

        self.system_manager
            .set_night_mode_enabled(value)
            .await
            .map_err(|e| {
                error!("Failed to enable/disable night mode, error {}", e);
                Status::internal("Failed to enable/disable night mode")
            })?;

        Ok(Response::new(()))
    }

    async fn set_nightmode_interval(
        &self,
        request: Request<TimeInterval>,
    ) -> Result<Response<()>, Status> {
        let request = request.into_inner();

        let from = parse_hhmm_to_naive_time(&request.from)?;
        let to = parse_hhmm_to_naive_time(&request.to)?;

        self.system_manager
            .set_night_mode_interval(from, to)
            .await
            .map_err(|e| {
                error!("Failed to set night mode interval, error: {}", e);
                Status::internal("Failed to set night mode interval")
            })?;

        Ok(tonic::Response::new(()))
    }
}

fn validate_brightness(value: u32) -> Result<(), Status> {
    if value > API_BRIGHTNESS_MAX {
        return Err(Status::invalid_argument(format!(
            "Invalid brightness. Value must be within a range [{API_BRIGHTNESS_MIN}-{API_BRIGHTNESS_MAX}]"
        )));
    }
    Ok(())
}

fn naive_time_to_hhmm(time: NaiveTime) -> String {
    time.format("%H:%M").to_string()
}

fn parse_hhmm_to_naive_time(input: &str) -> Result<NaiveTime, Status> {
    NaiveTime::parse_from_str(input, "%H:%M").map_err(|e| {
        warn!("Failed to parse Time in format HH:MM, error: {}", e);
        Status::invalid_argument("Invalid time format")
    })
}
