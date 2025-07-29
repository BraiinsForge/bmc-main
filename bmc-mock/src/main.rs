// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc::{BmcManager, log};
use bmc_mock::MockSessionManager;
use bmc_mock::{cli, manager::Manager, mock_index::MockIndex, mockfs};
use bmc_mock_display::VirtualDisplay;
use bmc_upgrade::firmware::FirmwareResolver;
use clap::Parser;
use std::sync::{Arc, Mutex};
use tokio::task::block_in_place;
use tooling_std::cancel::Cancel;

#[tokio::main]
async fn main() -> Result<()> {
    log::init();

    let config = cli::Config::parse();
    let system_password = config.system_password.clone();

    let mockfs = mockfs::MockFs::new(&config.mockfs_template, &config.mockfs_path);
    mockfs.init(
        config.mockfs_reset,
        config.factory_default,
        config.setup_pending,
    )?;

    let password = Arc::new(Mutex::new(system_password));

    let manager = Manager::new(
        mockfs,
        MockSessionManager::new(password.clone()),
        password,
        config.hostname.clone(),
        config.mac_address.clone(),
        config.ip_address,
        config.address.port(),
    );

    let config = config.into();

    let (main_window, display_driver) = VirtualDisplay::create()?;

    let firmware_resolver = FirmwareResolver::new(MockIndex);

    let job_scheduler = bmc_scheduler::JobScheduler::new(
        bmc_scheduler::JobSchedulerLocked::new().await?,
        manager.watch_timezone_updates(),
    );
    job_scheduler
        .init()
        .await
        .map_err(|_| anyhow::anyhow!("Failed to initialize job scheduler"))?;

    let main_join_handle = tokio::task::spawn({
        let display_controller = display_driver.display_controller.clone();
        let window_closed_fut = display_controller.window_closed();
        async move {
            let result = bmc::entry::main(manager, config, display_driver, firmware_resolver).await;
            display_controller.quit();
            result
        }
        .cancel(window_closed_fut)
    });

    block_in_place(move || main_window.run())?;

    match main_join_handle.await {
        Ok(Ok(main_result)) => {
            // main stopped
            main_result
        }
        Ok(Err(())) => {
            // window was closed
            Ok(())
        }
        Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
        Err(_) => unreachable!("BUG: main_join_handle aborted"),
    }
}
