// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::PathBuf;

pub use clap::Parser;

#[derive(Debug, Parser)]
pub struct Args {
    #[clap(long)]
    pub log_to_file: bool,

    /// Path to widget directories (can be specified multiple times)
    #[clap(long = "widgets-path")]
    pub widgets_paths: Vec<PathBuf>,
}
