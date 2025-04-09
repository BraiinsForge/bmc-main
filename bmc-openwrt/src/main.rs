// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc_openwrt::OpenwrtInitializer;

#[tokio::main]
async fn main() -> Result<()> {
    let initializer = OpenwrtInitializer {};

    bmc_core::entry::main(initializer).await
}
