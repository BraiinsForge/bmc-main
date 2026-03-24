// Copyright (C) 2026  Braiins Systems s.r.o.

pub mod app;
pub mod bos_upgrade;
pub mod config;
pub mod display;
pub mod init;
pub mod proxy;
pub mod server;
pub mod state;
pub mod utils;

#[expect(warnings)]
mod generated {
    slint::include_modules!();
}
pub use generated::*;
