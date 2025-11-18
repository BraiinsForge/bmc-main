// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::sound::Sounds;
use bmc_led::data::{LedEffectKind, Rgb};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct CountdownCompletionAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub led: Option<LedSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<SoundSettings>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct LedSettings {
    pub effect: LedEffectKind,
    pub color: Rgb,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SoundSettings {
    pub sound: Sounds,
    pub volume: u8,
}
