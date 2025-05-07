// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow as _;
use anyhow::Result;
use bmc::BmcManager;
use bmc_display as _;
use bmc_mock_display as _;
pub use session::MockSessionManager;
use slint as _;
use tokio as _;
use tokio as _;
use tracing::info;

pub mod cli;
pub mod manager;
pub mod mock_index;
pub mod mockfs;
mod session;

use bmc_display as _;
use bmc_mock_display as _;

use slint as _;

pub use mockfs::MockFs;
