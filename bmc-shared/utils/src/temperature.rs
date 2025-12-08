// Copyright (C) 2025  Braiins Systems s.r.o.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum TemperatureUnit {
    #[default]
    Celsius,
    Fahrenheit,
}
