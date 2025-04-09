// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use async_trait::async_trait;
use bmc_core::entry::Initializer;
use bmc_core::{BmcManager, Configuration, log};

pub struct OpenwrtInitializer;

#[async_trait]
impl Initializer for OpenwrtInitializer {
    async fn initialize(self) -> Result<(impl BmcManager, Configuration)> {
        log::init();

        let config = Configuration::default();
        Ok((OpenwrtManager, config))
    }
}

struct OpenwrtManager;

impl BmcManager for OpenwrtManager {
    fn version(&self) -> String {
        "Hello from Openwrt".to_string()
    }
}
