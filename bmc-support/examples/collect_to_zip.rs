// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc_support::SupportArchiveFormat;
use std::env;
use std::fs::File;

fn main() -> Result<()> {
    // set RUST_LOG to 'info' if unset
    if env::var_os("RUST_LOG").is_none() {
        unsafe {
            env::set_var("RUST_LOG", "info");
        }
    }

    tracing_subscriber::fmt::init();

    let mut file = File::create("support_archive.zip")?;
    bmc_support::collect(&mut file, SupportArchiveFormat::ZipEncrypted, false)?;

    println!("{file:?}");

    Ok(())
}
