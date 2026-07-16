// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use std::{path::PathBuf, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::config::ConfigHandle;
use bmc_audio::{Audio, Volume};

const SLEEP_DURATION: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub(crate) struct SoundController {
    config_handle: Arc<RwLock<ConfigHandle>>,
    sounds_dir: PathBuf,
    audio: Arc<RwLock<Audio>>,
}

impl SoundController {
    pub(crate) fn new(config_handle: Arc<RwLock<ConfigHandle>>, sounds_dir: PathBuf) -> Self {
        Self {
            config_handle,
            sounds_dir,
            audio: Arc::new(RwLock::new(Audio::new())),
        }
    }

    pub(crate) async fn sound_volume(&self) -> u8 {
        self.config_handle.read().await.sound_volume_pct()
    }

    pub(crate) async fn set_config_sound_volume(&self, value: u8) -> anyhow::Result<()> {
        let mut config_handle = self.config_handle.write().await;
        config_handle.set_sound_volume(value);

        config_handle.save().await?;

        info!(volume_pct = value, "Sound volume configuration updated");

        Ok(())
    }

    pub(crate) async fn set_audio_sound_volume(&self, value: u8) -> anyhow::Result<()> {
        self.audio.write().await.set_volume(Volume::new(value)?);

        info!(volume_pct = value, "Audio sound volume applied");

        Ok(())
    }

    pub(crate) async fn play_sound(
        &self,
        sound: Sounds,
        token: CancellationToken,
    ) -> anyhow::Result<()> {
        let path = self.sounds_dir.join(sound.file_name());
        let audio = self.audio.read().await;
        audio.play(path.as_os_str(), token).await
    }

    pub(crate) async fn play_until_cancelled(&self, sound: Sounds, token: CancellationToken) {
        while !token.is_cancelled() {
            if let Err(err) = self.play_sound(sound.clone(), token.clone()).await {
                warn!(
                    error = %err,
                    sound = %sound,
                    "Failed to play sound"
                );
                //NOTE: Sleep for few seconds. Resource can by busy playing other sounds
                tokio::time::sleep(SLEEP_DURATION).await;
            }
        }
    }
}

#[derive(Debug, Clone, Display, EnumIter, EnumString, Serialize, Deserialize)]
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
            Sounds::GreenCandleMorning => "green_candle_morning.mp3",
            Sounds::KeepCalmAndDca => "keep_calm_and_dca.mp3",
            Sounds::KeepCalmAndDcaV2 => "keep_calm_and_dca_v2.mp3",
            Sounds::HashrateMelody => "hashrate_melody.mp3",
            Sounds::TickTockNextBlock => "tick_tock_next_block.mp3",
            Sounds::OgStyleWakingUp => "og_style_waking_up.mp3",
            Sounds::SonarPriceAlert => "sonar_price_alert.mp3",
            Sounds::SubtlePriceAlert => "subtle_price_alert.mp3",
            Sounds::PriceUp => "price_up.mp3",
            Sounds::PriceDown => "price_down.mp3",
            Sounds::Confirmation => "confirmation.mp3",
            Sounds::ErrorSound => "error_sound.mp3",
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
