// Copyright (C) 2025  Braiins Systems s.r.o.

pub mod cli;
pub mod mockfs;

use anyhow as _;
use bmc::BmcManager;
use bmc_display as _;
use bmc_mock_display as _;
use slint as _;
use tokio as _;

#[derive(Debug)]
pub struct MockManager;

impl BmcManager for MockManager {
    fn version(&self) -> String {
        "Hello from Mock".to_owned()
    }
}
