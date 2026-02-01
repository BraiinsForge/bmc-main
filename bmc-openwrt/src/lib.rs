// Copyright (C) 2025  Braiins Systems s.r.o.

pub mod button_driver;
pub mod cli;
pub mod generic_backlight_driver;
pub mod led_driver;
pub mod linux_drm_platform;
pub mod log;
pub mod manager;
mod pwd;
pub mod session;
mod signal;
mod sys;
pub mod uboot_env;
mod unix;

const ROOT_USERNAME: &str = "root";
