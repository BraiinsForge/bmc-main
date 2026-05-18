// Copyright (C) 2026  Braiins Systems s.r.o.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum UnitSystem {
    #[default]
    Metric,
    Imperial,
}
