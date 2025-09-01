// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::BmcManager;
use crate::initial_setup::InitSetupState;
use crate::system_upgrade::SystemUpgradeState;

use crate::config::ConfigHandle;
use bmc_display::bitcoin_data::BitcoinData;
use bmc_display::blockheight_data::BlockheightData;
use bmc_display::data::Screen;
use bmc_display::display_controller::DisplayController;
use bmc_shared_time::time::Timezone;
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, watch};
use tokio::time::interval;
use tracing::{debug, info, warn};

const SCREEN_DURATION: Duration = Duration::from_secs(5);

const PRICE_API_URL: &str = "https://public-api.braiins.com/v1/price-stats";
const BLOCK_HEIGHT_API_URL: &str = "https://public-api.braiins.com/v2/blocks";
const BLOCK_HEIGHT_LIMIT_API_PARAM: &str = "limit";
const CURRENCY_API_PARAM: &str = "currency";
const API_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub(crate) struct DisplayTasks<T: BmcManager> {
    display_controller: DisplayController,
    system_upgrade_receiver: watch::Receiver<Option<SystemUpgradeState>>,
    timezone_receiver: watch::Receiver<Timezone>,
    initial_setup_receiver: watch::Receiver<Option<InitSetupState>>,
    manager: Arc<T>,
    config_handle: Arc<RwLock<ConfigHandle>>,
}

impl<T: BmcManager> DisplayTasks<T> {
    pub(crate) fn new(
        display_controller: DisplayController,
        system_upgrade_receiver: watch::Receiver<Option<SystemUpgradeState>>,
        timezone_receiver: watch::Receiver<Timezone>,
        initial_setup_receiver: watch::Receiver<Option<InitSetupState>>,
        manager: Arc<T>,
        config_handle: Arc<RwLock<ConfigHandle>>,
    ) -> Self {
        Self {
            display_controller,
            system_upgrade_receiver,
            timezone_receiver,
            initial_setup_receiver,
            manager,
            config_handle,
        }
    }

    pub(crate) fn spawn(self) {
        let Self {
            display_controller,
            system_upgrade_receiver,
            timezone_receiver,
            initial_setup_receiver,
            manager,
            config_handle,
        } = self;

        tokio::spawn(Self::run_init_display_screen(
            display_controller.clone(),
            manager.clone(),
        ));

        tokio::spawn(Self::run_system_upgrade_listener(
            display_controller.clone(),
            system_upgrade_receiver,
        ));

        tokio::spawn(Self::run_timezone_listener(timezone_receiver));

        tokio::spawn(Self::run_price_update(display_controller.clone()));

        tokio::spawn(Self::run_blockheight_update(
            display_controller.clone(),
            config_handle.clone(),
            manager.clone(),
        ));

        tokio::spawn(Self::run_initial_setup_listener(
            display_controller,
            manager,
            initial_setup_receiver,
        ));
    }

    async fn run_system_upgrade_listener(
        display_controller: DisplayController,
        mut receiver: watch::Receiver<Option<SystemUpgradeState>>,
    ) {
        while let Ok(()) = receiver.changed().await {
            let Some(upgrade_state) = &*receiver.borrow_and_update() else {
                continue;
            };

            match upgrade_state {
                SystemUpgradeState::DownloadStarted { total_mb } => {
                    display_controller.update_download_firmware_progress(0.0, *total_mb);
                    display_controller.set_screen(Screen::DownloadFirmware);
                }
                SystemUpgradeState::DownloadProgress {
                    downloaded_mb,
                    total_mb,
                } => {
                    display_controller.update_download_firmware_progress(*downloaded_mb, *total_mb);
                }
                SystemUpgradeState::DownloadFinished { total_mb, .. } => {
                    display_controller.update_download_firmware_progress(*total_mb, *total_mb);
                }
                SystemUpgradeState::UpgradeStarted => {
                    display_controller.set_screen(Screen::Upgrade);
                }
                SystemUpgradeState::Failed => {
                    display_controller.set_screen(Screen::UpgradeFailed);
                }
            }
        }
    }

    async fn run_timezone_listener(mut receiver: watch::Receiver<Timezone>) {
        while let Ok(()) = receiver.changed().await {
            let timezone = receiver.borrow_and_update();
            info!(?timezone, "Timezone was changed");
        }
    }

    async fn run_initial_setup_listener(
        display_controller: DisplayController,
        manager: Arc<T>,
        mut receiver: watch::Receiver<Option<InitSetupState>>,
    ) {
        while let Ok(()) = receiver.changed().await {
            let state = (*receiver.borrow_and_update()).clone();
            info!(?state, "Initial setup state was changed");
            if let Some(initial_setup_state) = state {
                match initial_setup_state {
                    InitSetupState::ConnectingToWifi { wifi_ssid } => {
                        display_controller.set_wifi_ssid(wifi_ssid);
                        display_controller.set_screen(Screen::InitialSetupWifiConnecting);
                    }
                    InitSetupState::WifiConnectionSuccess => {
                        display_controller.set_screen(Screen::InitialSetupWifiConnected);
                        tokio::time::sleep(SCREEN_DURATION).await;
                        let ip = manager.ip_address().await;
                        display_controller.set_connect_ip_qr_code(ip);
                        display_controller.set_screen(Screen::InitialSetupConnectInfo);
                    }
                    InitSetupState::WifiConnectionFailed => {
                        display_controller.set_screen(Screen::InitialSetupWifiError);
                        tokio::time::sleep(SCREEN_DURATION).await;
                        let ssid = manager.wifi_ssid();
                        display_controller.set_wifi_ssid(ssid);
                        display_controller.set_screen(Screen::InitialSetupStart);
                    }
                    InitSetupState::UnexpectedError => {
                        display_controller.set_screen(Screen::InitialSetupGeneralError);
                    }
                    InitSetupState::DeviceSetupSuccess => {
                        display_controller.set_screen(Screen::InitialSetupCompleted);
                        tokio::time::sleep(SCREEN_DURATION).await;
                        display_controller.set_screen(Screen::Void);
                    }
                }
            }
        }
    }

    async fn run_init_display_screen(display_controller: DisplayController, manager: Arc<T>) {
        if manager.check_and_remove_upgrade_marker().await {
            display_controller.set_screen(Screen::UpgradeSuccess);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }

        let state = manager.device_state().await;

        match state {
            crate::manager::BmcState::FactoryDefault => {
                let ssid = manager.wifi_ssid();
                display_controller.set_wifi_ssid(ssid);
                display_controller.set_screen(Screen::InitialSetupStart);
            }
            crate::manager::BmcState::SetupPending => {
                let ip = manager.ip_address().await;
                display_controller.set_connect_ip_qr_code(ip);
                display_controller.set_screen(Screen::InitialSetupConnectInfo);
            }
            crate::manager::BmcState::Operational => {
                let ip = manager.ip_address().await;
                display_controller.set_connect_ip_qr_code(ip);
                display_controller.set_screen(Screen::ConnectInfo);
                tokio::time::sleep(Duration::from_secs(10)).await;
                display_controller.set_screen(Screen::Void);
            }
        }
    }

    async fn run_price_update(display_controller: DisplayController) {
        let mut interval = interval(Duration::from_secs(60));

        loop {
            interval.tick().await;

            debug!("Getting bitcoin data...");
            let client = Client::new();
            let btc_price_data =
                if let Ok(response) = client.get(PRICE_API_URL).timeout(API_TIMEOUT).send().await {
                    response.json::<BitcoinData>().await.unwrap_or_default()
                } else {
                    warn!("Failed to get bitcoin data from API");
                    BitcoinData::default()
                };

            display_controller.update_btc_price(btc_price_data);
        }
    }

    async fn run_blockheight_update(
        display_controller: DisplayController,
        config_handle: Arc<RwLock<ConfigHandle>>,
        manager: Arc<T>,
    ) {
        let mut interval = interval(Duration::from_secs(60));

        loop {
            interval.tick().await;

            debug!("Getting blockheight data...");
            let client = Client::new();
            let blockheight_data = if let Ok(response) = client
                .get(BLOCK_HEIGHT_API_URL)
                .query(&[
                    (BLOCK_HEIGHT_LIMIT_API_PARAM, "1"),
                    (CURRENCY_API_PARAM, "usd"),
                ])
                .timeout(API_TIMEOUT)
                .send()
                .await
            {
                response
                    .json::<Vec<BlockheightData>>()
                    .await
                    .unwrap_or_default()
                    .first()
                    .cloned()
                    .unwrap_or_default()
            } else {
                warn!("Failed to get blockheight data from API");
                BlockheightData::default()
            };

            let timezone = manager.timezone();

            let is_24_format = config_handle
                .read()
                .await
                .localization_config()
                .time_system
                .is_24();
            let date_format = config_handle.read().await.localization_config().date_format;

            display_controller.update_blockheight_data(
                blockheight_data,
                timezone,
                is_24_format,
                date_format,
            );
        }
    }
}
