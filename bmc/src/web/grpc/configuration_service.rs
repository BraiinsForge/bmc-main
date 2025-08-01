// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::backlight::DisplayBacklightController;
use bmc_display::display_driver::DisplayBacklightDriver;
use bmc_grpc::web;
use tonic::{Request, Response, Status};
use tracing::warn;

const API_BRIGHTNESS_MIN: u32 = 0;
const API_BRIGHTNESS_MAX: u32 = 100;
const API_BRIGHTNESS_STEP: u32 = 5;

pub(crate) struct ConfigurationService<T: DisplayBacklightDriver> {
    backlight_controller: DisplayBacklightController<T>,
}

impl<T: DisplayBacklightDriver> ConfigurationService<T> {
    pub(crate) fn new(backlight_controller: DisplayBacklightController<T>) -> Self {
        Self {
            backlight_controller,
        }
    }
}

#[async_trait::async_trait]
impl<T: DisplayBacklightDriver> web::configuration_service_server::ConfigurationService
    for ConfigurationService<T>
{
    async fn set_brightness(&self, request: Request<u32>) -> Result<Response<()>, Status> {
        let value = request.into_inner();

        if value > API_BRIGHTNESS_MAX {
            return Err(Status::invalid_argument(format!(
                "Invalid brightness. Value must be within a range [{API_BRIGHTNESS_MIN}-{API_BRIGHTNESS_MAX}]"
            )));
        }
        #[expect(clippy::cast_possible_truncation)]
        self.backlight_controller
            .set_brightness_pct(value as u8)
            .await
            .map_err(|e| {
                warn!("Cannot set display brightness: {}", e);
                Status::internal("Failed to set display brightness")
            })?;

        Ok(Response::new(()))
    }

    async fn get_display_settings(
        &self,
        _request: Request<()>,
    ) -> Result<Response<web::DisplaySettingsResponse>, Status> {
        let value = u32::from(self.backlight_controller.get_brightness_pct().await);

        Ok(Response::new(web::DisplaySettingsResponse {
            brightness: Some(web::BrightnessInfo {
                value,
                min: API_BRIGHTNESS_MIN,
                max: API_BRIGHTNESS_MAX,
                step: API_BRIGHTNESS_STEP,
            }),
        }))
    }
}
