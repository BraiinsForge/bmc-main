// Copyright (C) 2023  Braiins Systems s.r.o.

pub mod antminer;
pub mod bos;
pub mod commit;

use enum_assoc::Assoc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter};

// NOTE: This enum deliberately does not contain `Recovery` and `Upgrade` modes
// NOTE: lowercase aliases are used for vnish info response deserialization
#[derive(
    Serialize,
    Deserialize,
    JsonSchema,
    Display,
    EnumIter,
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
)]
pub enum BootMode {
    #[strum(serialize = "eMMC")]
    #[serde(rename = "eMMC", alias = "emmc")]
    Emmc,
    #[strum(serialize = "NAND")]
    #[serde(rename = "NAND", alias = "nand")]
    Nand,
    #[strum(serialize = "SD")]
    #[serde(rename = "SD", alias = "sd")]
    Sd,
}

#[derive(
    Serialize,
    Deserialize,
    JsonSchema,
    Display,
    EnumIter,
    Assoc,
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
)]
#[func(pub const fn arch(&self) -> Arch)]
pub enum ControlBoard {
    /// Zynq (Antminer)
    #[assoc(arch = Arch::ARMv7)]
    Zynq,
    /// BeagleBone (Antminer)
    #[assoc(arch = Arch::ARMv7)]
    BBB,
    /// AmLogic (Antminer)
    #[assoc(arch = Arch::AArch64)]
    AML,
    /// CVITEK (Antminer)
    #[assoc(arch = Arch::AArch64)]
    CVITEK,
    /// Braiins CB
    #[assoc(arch = Arch::ARMv7)]
    Braiins,
    /// H3 (WhatsMiner)
    #[assoc(arch = Arch::ARMv7)]
    H3,
    /// H6 (WhatsMiner)
    #[assoc(arch = Arch::ARMv8)]
    H6,
    /// H6OS (WhatsMiner)
    #[assoc(arch = Arch::ARMv8)]
    H6OS,
    /// H616 (WhatsMiner)
    #[assoc(arch = Arch::ARMv8)]
    H616,
}

#[derive(Display, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum Arch {
    ARMv7,
    ARMv8,
    AArch64,
}

#[derive(
    Serialize,
    Deserialize,
    JsonSchema,
    Display,
    Debug,
    Copy,
    Clone,
    Eq,
    PartialEq,
    PartialOrd,
    Ord,
    Hash,
)]
pub enum Firmware {
    #[strum(serialize = "Braiins OS")]
    BOS,
    #[strum(serialize = "Bitmain Stock")]
    BitmainStock,
    #[strum(serialize = "MicroBT Stock")]
    MicroBTStock,
    #[strum(serialize = "IceRiver Stock")]
    IceRiverStock,
    #[strum(serialize = "Canaan Stock")]
    CanaanStock,
    #[strum(serialize = "Vnish")]
    Vnish,
    #[strum(serialize = "LuxOS")]
    LuxOs,
    #[strum(serialize = "Virtual")]
    Virtual,
    #[strum(serialize = "Unknown")]
    Unknown,
}
