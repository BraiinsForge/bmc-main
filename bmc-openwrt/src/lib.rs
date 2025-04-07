// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc_core::{BmcManager, Configuration, log};

pub fn init() -> Result<(impl BmcManager, Configuration)> {
    log::init();

    let config = Configuration::default();
    Ok((Manager, config))
}

pub fn get_manager() -> impl BmcManager {
    Manager
}

struct Manager;

impl BmcManager for Manager {
    fn version(&self) -> String {
        "Hello from Openwrt".to_string()
    }
}
