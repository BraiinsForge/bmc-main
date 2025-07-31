// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::sound::Sounds;
use crate::system_manager::SystemManager;

use bmc_display::display_driver::DisplayBacklightDriver;
use bmc_grpc::web::{
    BrightnessInfo, DisplaySettingsResponse, SoundVolumeSettingsResponse, TimeInterval,
    configuration_service_server::ConfigurationService as GrpcConfigurationService,
};
use bmc_grpc::web::{ListSoundsResponse, PlaySoundRequest, SoundInfo, SoundVolume};
use std::str::FromStr;
use strum::IntoEnumIterator;
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status};
use tracing::{error, warn};

use super::SoundController;
use super::shared::{naive_time_to_hhmm, parse_hhmm_to_naive_time};

const API_BRIGHTNESS_MIN: u32 = 0;
const API_BRIGHTNESS_MAX: u32 = 100;
const API_BRIGHTNESS_STEP: u32 = 5;

const API_SOUND_VOLUME_MIN: u32 = 0;
const API_SOUND_VOLUME_MAX: u32 = 100;
const API_SOUND_VOLUME_STEP: u32 = 1;

pub(crate) struct ConfigurationService<T: DisplayBacklightDriver> {
    system_manager: SystemManager<T>,
    sound_controller: SoundController,
}

impl<T: DisplayBacklightDriver> ConfigurationService<T> {
    pub(crate) fn new(system_manager: SystemManager<T>, sound_controller: SoundController) -> Self {
        Self {
            system_manager,
            sound_controller,
        }
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

    async fn get_sound_volume_settings(
        &self,
        _request: Request<()>,
    ) -> Result<Response<SoundVolumeSettingsResponse>, Status> {
        let sound_settings = self.system_manager.sound_settings().await;
        Ok(Response::new(SoundVolumeSettingsResponse {
            volume: Some(SoundVolume {
                value: u32::from(sound_settings.volume),
                min: API_SOUND_VOLUME_MIN,
                max: API_SOUND_VOLUME_MAX,
                step: API_SOUND_VOLUME_STEP,
            }),
            volume_nightmode: Some(SoundVolume {
                value: u32::from(sound_settings.volume_night_mode),
                min: API_SOUND_VOLUME_MIN,
                max: API_SOUND_VOLUME_MAX,
                step: API_SOUND_VOLUME_STEP,
            }),
        }))
    }

    async fn set_sound_volume(&self, request: Request<u32>) -> Result<Response<()>, Status> {
        let value = request.into_inner();

        validate_sound_volume(value)?;

        #[expect(clippy::cast_possible_truncation)]
        self.system_manager
            .set_sound_volume(value as u8)
            .await
            .map_err(|e| {
                warn!("Failed to set sound volume, error: {}", e);
                Status::internal("Failed to set sound volume")
            })?;

        Ok(Response::new(()))
    }

    async fn set_sound_volume_nightmode(
        &self,
        request: Request<u32>,
    ) -> Result<Response<()>, Status> {
        let value = request.into_inner();

        validate_sound_volume(value)?;

        #[expect(clippy::cast_possible_truncation)]
        self.system_manager
            .set_sound_volume_night_mode(value as u8)
            .await
            .map_err(|e| {
                warn!("Failed to set sound volume for night mode, error: {}", e);
                Status::internal("Failed to set sound volume for night mode")
            })?;

        Ok(Response::new(()))
    }

    async fn list_sounds(
        &self,
        _request: Request<()>,
    ) -> Result<Response<ListSoundsResponse>, Status> {
        let sounds: Vec<SoundInfo> = Sounds::iter().map(Into::into).collect();

        Ok(Response::new(ListSoundsResponse { sounds }))
    }

    async fn play_sound(&self, request: Request<PlaySoundRequest>) -> Result<Response<()>, Status> {
        let sound = Sounds::from_str(&request.into_inner().sound_id).map_err(|e| {
            warn!("Failed to parse sound, error {e}");
            Status::invalid_argument("Invalid sound_id")
        })?;

        let token = CancellationToken::new();
        // NOTE: _drop_guard works here as an automatic cancellation of the sound playing.
        // If user cancels the operation e.g. cancels gRPC request, _drop_guard is dropped which cancels the token
        // which cancels the process of sound play.
        let _drop_guard = token.clone().drop_guard();

        let sound_controller = self.sound_controller.clone();

        // NOTE: Spawn `Audio::play` to avoid blocking cancellation;
        // running it inline would delay dropping the token and prevent cancellation.
        tokio::spawn(async move {
            sound_controller
                .play_sound(sound, token)
                .await
                .map_err(|e| {
                    error!("Failed to play sound, error {e}");
                    Status::internal("Failed to play sound")
                })?;

            Ok::<(), Status>(())
        })
        .await
        .map_err(|_| Status::internal("Failed to play sound"))??;

        Ok(Response::new(()))
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

fn validate_sound_volume(value: u32) -> Result<(), Status> {
    if value > API_SOUND_VOLUME_MAX {
        return Err(Status::invalid_argument(format!(
            "Invalid sound volume. Value must be within a range [{API_SOUND_VOLUME_MIN}-{API_SOUND_VOLUME_MAX}]"
        )));
    }
    Ok(())
}

impl From<Sounds> for SoundInfo {
    fn from(value: Sounds) -> Self {
        Self {
            id: value.to_string(),
            name: value.name().to_owned(),
        }
    }
}
