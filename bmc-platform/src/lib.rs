// Copyright (C) 2025  Braiins Systems s.r.o.

use std::fmt::Display;

use index_bmc::BmcPlatform as IndexBmcPlatform;
use strum::{EnumIter, EnumMessage, EnumString};

#[derive(Debug, Clone, Copy, EnumString, Eq, PartialEq, EnumMessage, EnumIter, Hash)]
pub enum BmcPlatform {
    #[strum(serialize = "stm32mp157c-ii3-bmc1")]
    BraiinsBmc,
}

impl Display for BmcPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Write first serialization, if available (should be always true).
        // In an unlikely case of no serialization, report the platform as unknown.
        write!(
            f,
            "{}",
            self.get_serializations().first().unwrap_or(&"unknown")
        )
    }
}

impl From<BmcPlatform> for IndexBmcPlatform {
    fn from(value: BmcPlatform) -> Self {
        match value {
            BmcPlatform::BraiinsBmc => IndexBmcPlatform::Stm32mp157cIi3Bmc1,
        }
    }
}
