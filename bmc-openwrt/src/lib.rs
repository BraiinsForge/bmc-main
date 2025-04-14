// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc::BmcManager;

pub mod generic_backlight_driver;
pub mod linux_framebuffer_platform;

pub struct OpenwrtManager;

impl BmcManager for OpenwrtManager {
    fn version(&self) -> String {
        "Hello from Openwrt".to_string()
    }
}
