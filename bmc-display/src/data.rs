// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated::{ClockLarge, ClockMedium, ClockSmall};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum WidgetType {
    ClockSmall(ClockSmall),
    ClockMedium(ClockMedium),
    ClockLarge(ClockLarge),
}
