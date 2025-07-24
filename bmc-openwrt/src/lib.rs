// Copyright (C) 2025  Braiins Systems s.r.o.

pub mod button_driver;
pub mod generic_backlight_driver;
pub mod led_driver;
pub mod linux_drm_platform;
pub mod manager;
mod pwd;
pub mod session;
mod sys;
mod unix;

const ROOT_USERNAME: &str = "root";
