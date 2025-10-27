// Copyright (C) 2025  Braiins Systems s.r.o.

pub use clap::Parser;

#[derive(Debug, Parser)]
pub struct Args {
    #[clap(long)]
    pub log_to_file: bool,
}
