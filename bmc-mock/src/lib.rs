// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc::BmcManager;

pub mod cli;
pub mod mockfs;

pub struct MockManager;

impl BmcManager for MockManager {
    fn version(&self) -> String {
        "Hello from Mock".to_string()
    }
}
