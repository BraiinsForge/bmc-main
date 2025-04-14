// Copyright (C) 2025  Braiins Systems s.r.o.

pub mod generic_backlight_driver;
pub mod linux_framebuffer_platform;

use bmc::BmcManager;
use tokio as _;

#[derive(Debug)]
pub struct OpenwrtManager;

impl BmcManager for OpenwrtManager {
    fn version(&self) -> String {
        "Hello from Openwrt".to_owned()
    }
}
