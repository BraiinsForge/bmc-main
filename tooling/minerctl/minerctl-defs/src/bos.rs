// Copyright (C) 2023  Braiins Systems s.r.o.

pub mod version;

use crate::ControlBoard;
use enum_assoc::Assoc;
use strum::{Display, EnumString};

#[derive(EnumString, Display, Assoc, Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[func(pub const fn control_board(&self) -> ControlBoard)]
pub enum BosPlatform {
    #[strum(serialize = "am1-s9")]
    #[assoc(control_board = ControlBoard::Zynq)]
    Am1s9,
    #[strum(serialize = "am2-s17")]
    #[assoc(control_board = ControlBoard::Zynq)]
    Am2s17,
    #[strum(serialize = "zynq-bm3-am2")]
    #[assoc(control_board = ControlBoard::Zynq)]
    ZynqBm3Am2,
    #[strum(serialize = "am3-bbb")]
    #[assoc(control_board = ControlBoard::BBB)]
    Am3bbb,
    #[strum(serialize = "am3-aml")]
    #[assoc(control_board = ControlBoard::AML)]
    Am3aml,
    #[strum(serialize = "wm1-h3")]
    #[assoc(control_board = ControlBoard::H3)]
    Wm1h3,
    #[strum(serialize = "wm1-h6")]
    #[assoc(control_board = ControlBoard::H6)]
    Wm1h6,
    #[strum(serialize = "wm1-h6os")]
    #[assoc(control_board = ControlBoard::H6OS)]
    Wm1h6os,
    #[strum(serialize = "stm32mp157c-ii1-am2")]
    #[assoc(control_board = ControlBoard::Braiins)]
    Stm32mp157cIi1Am2,
    #[strum(serialize = "stm32mp157c-ii2-bmm1")]
    #[assoc(control_board = ControlBoard::Braiins)]
    Stm32mp157cIi2Bmm1,
    #[strum(serialize = "cvitek-bm1-am2")]
    #[assoc(control_board = ControlBoard::CVITEK)]
    CvitekBm1Am2,
}
