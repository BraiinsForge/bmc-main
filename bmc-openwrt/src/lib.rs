// Copyright (C) 2025  Braiins Systems s.r.o.

pub mod generic_backlight_driver;
pub mod linux_framebuffer_platform;
pub mod manager;
mod pwd;
pub mod session;
mod sys;
mod unix;

use bmc_upgrade as _;
use iana_time_zone as _;
use tokio as _;

const ROOT_USERNAME: &str = "root";
