// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc_mock::MockInitializer;

#[tokio::main]
async fn main() -> Result<()> {
    let initializer = MockInitializer {};

    bmc_core::entry::main(initializer).await
}
