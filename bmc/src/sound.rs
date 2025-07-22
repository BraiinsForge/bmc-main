// Copyright (C) 2025  Braiins Systems s.r.o.

use std::{path::PathBuf, sync::Arc};

use strum::{Display, EnumIter, EnumString};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::error;

use crate::config::ConfigHandle;
use bmc_audio::Audio;

#[derive(Clone, Debug)]
pub(crate) struct SoundController {
    config_handle: Arc<RwLock<ConfigHandle>>,
    sounds_dir: PathBuf,
}

impl SoundController {
    pub(crate) fn new(config_handle: Arc<RwLock<ConfigHandle>>, sounds_dir: PathBuf) -> Self {
        Self {
            config_handle,
            sounds_dir,
        }
    }

    pub(crate) async fn sound_volume(&self) -> u8 {
        self.config_handle.read().await.sound_volume_pct()
    }

    pub(crate) async fn set_config_sound_volume(&self, value: u8) -> anyhow::Result<()> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_sound_volume(value);
        let config_handle = config_handle.downgrade();

        config_handle.sync_to_storage().await?;

        Ok(())
    }

    pub(crate) async fn set_audio_sound_volume(&self, value: u8) -> anyhow::Result<()> {
        Audio::set_volume(value)
            .await
            .inspect_err(|e| error!("Failed to set system sound volume to {value}, error: {e}"))
    }

    pub(crate) async fn play_sound(
        &self,
        sound: Sounds,
        token: CancellationToken,
    ) -> anyhow::Result<()> {
        let path = self.sounds_dir.join(sound.file_name());

        // NOTE: Spawn `Audio::play` to avoid blocking cancellation;
        // running it inline would delay dropping the token and prevent cancellation.
        tokio::spawn(async move { Audio::play(path.as_os_str(), token).await }).await?
    }
}

#[derive(Debug, Clone, Display, EnumIter, EnumString)]
#[strum(serialize_all = "PascalCase")]
pub(crate) enum Sounds {
    GreenCandleMorning,
    KeepCalmAndDca,
    KeepCalmAndDcaV2,
    HashrateMelody,
    TickTockNextBlock,
    OgStyleWakingUp,
    SonarPriceAlert,
    SubtlePriceAlert,
    PriceUp,
    PriceDown,
    Confirmation,
    ErrorSound,
}

impl Sounds {
    pub(crate) fn file_name(&self) -> &'static str {
        match self {
            Sounds::GreenCandleMorning => "just-sound-effects-smartphone-ui-synth-clock-alarm.mp3",
            Sounds::KeepCalmAndDca => "ni-sound-emotion-loops-puzzle-solving-music-mysterious.mp3",
            Sounds::KeepCalmAndDcaV2 => {
                "just-sound-effects-smartphone-ui-strings-and-pops-clock-alarm.mp3"
            }
            Sounds::HashrateMelody => "just-sound-effects-smartphone-ui-plucks-ringtone.mp3",
            Sounds::TickTockNextBlock => "federico-soler-clock-suspenseful-creepy-distorted.mp3",
            Sounds::OgStyleWakingUp => {
                "just-sound-effects-smartphone-ui-common-alert-clock-alarm.mp3"
            }
            Sounds::SonarPriceAlert => "ni-sound-cute-interface-synth-bleep-notification.mp3",
            Sounds::SubtlePriceAlert => {
                "airborne-sound-app-and-menu-alert-or-notification-subtle-panning-clean-tone.mp3"
            }
            Sounds::PriceUp => "bmc-price-up-sound.mp3",
            Sounds::PriceDown => "bmc-price-down-sound.mp3",
            Sounds::Confirmation => "stuart-duffield-menu-select-reverberant-beep.mp3",
            Sounds::ErrorSound => "stuart-duffield-menu-error-synth-bass-double-note.mp3",
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            Sounds::GreenCandleMorning => "Green candle morning",
            Sounds::KeepCalmAndDca => "Keep calm and DCA",
            Sounds::KeepCalmAndDcaV2 => "Keep calm and DCA V2",
            Sounds::HashrateMelody => "Hashrate melody",
            Sounds::TickTockNextBlock => "Tick tock, next block",
            Sounds::OgStyleWakingUp => "OG style waking up",
            Sounds::SonarPriceAlert => "Sonar price alert",
            Sounds::SubtlePriceAlert => "Subtle price alert",
            Sounds::PriceUp => "Price up",
            Sounds::PriceDown => "Price down",
            Sounds::Confirmation => "Confirmation",
            Sounds::ErrorSound => "Error Sound",
        }
    }
}
