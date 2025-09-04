// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::BmcManager;
use crate::alarm::AlarmBus;
use crate::initial_setup::InitSetupState;
use crate::system_upgrade::SystemUpgradeState;

use crate::config::ConfigHandle;
use bmc_display::bitcoin_data::BitcoinData;
use bmc_display::blockheight_data::BlockheightData;
use bmc_display::data::Screen;
use bmc_display::display_controller::DisplayController;
use bmc_shared_ii_net::wifi::SignalStrength;
use bmc_shared_time::time::Timezone;
use futures::StreamExt;
use reqwest::Client;
use std::net::IpAddr;
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
const CONNECT_INFO_SCREEN_DURATION: Duration = Duration::from_secs(10);
const ERROR_SCREEN_DURATION: Duration = Duration::from_secs(5);
const CHECK_IP_ATTEMPTS: u8 = 10;
const CHECK_IP_WAIT_DURATION: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub(crate) struct DisplayTasks<T: BmcManager> {
    display_controller: DisplayController,
    system_upgrade_receiver: watch::Receiver<Option<SystemUpgradeState>>,
    timezone_receiver: watch::Receiver<Timezone>,
    initial_setup_receiver: watch::Receiver<Option<InitSetupState>>,
    manager: Arc<T>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    alarm_bus: AlarmBus,
}

impl<T: BmcManager> DisplayTasks<T> {
    pub(crate) fn new(
        display_controller: DisplayController,
        system_upgrade_receiver: watch::Receiver<Option<SystemUpgradeState>>,
        timezone_receiver: watch::Receiver<Timezone>,
        initial_setup_receiver: watch::Receiver<Option<InitSetupState>>,
        manager: Arc<T>,
        config_handle: Arc<RwLock<ConfigHandle>>,
        alarm_bus: AlarmBus,
    ) -> Self {
        Self {
            display_controller,
            system_upgrade_receiver,
            timezone_receiver,
            initial_setup_receiver,
            manager,
            config_handle,
            alarm_bus,
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
            alarm_bus,
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

        tokio::spawn(Self::run_wifi_offline_check(
            display_controller.clone(),
            manager.clone(),
        ));

        tokio::spawn(Self::run_price_update(display_controller.clone()));

        tokio::spawn(Self::run_blockheight_update(
            display_controller.clone(),
            config_handle.clone(),
            manager.clone(),
        ));

        tokio::spawn(Self::run_date_time_update(
            display_controller.clone(),
            config_handle.clone(),
            manager.clone(),
        ));

        tokio::spawn(Self::run_initial_setup_listener(
            display_controller.clone(),
            manager,
            initial_setup_receiver,
        ));

        tokio::spawn(Self::run_alarm_event_listener(
            display_controller,
            alarm_bus,
        ));
    }

    async fn run_date_time_update(
        display_controller: DisplayController,
        config_handle: Arc<RwLock<ConfigHandle>>,
        manager: Arc<T>,
    ) {
        let mut interval = interval(Duration::from_millis(250));

        loop {
            interval.tick().await;

            let timezone = manager.timezone();
            let now = chrono::Local::now()
                .with_timezone(timezone.chrono())
                .fixed_offset();

            let is_24_format = config_handle
                .read()
                .await
                .localization_config()
                .time_system
                .is_24();

            display_controller.update_system_datetime(now, timezone.to_string(), is_24_format);
        }
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

    async fn run_wifi_offline_check(display_controller: DisplayController, manager: Arc<T>) {
        let mut interval = interval(Duration::from_secs(5));

        loop {
            interval.tick().await;

            let is_offline = match manager.wifi_status().await {
                Ok(wifi) => {
                    let signal_strength = wifi
                        .status
                        .sta_link_state
                        .unwrap_or_default()
                        .signal_strength();

                    signal_strength == SignalStrength::Offline
                }
                Err(err) => {
                    warn!(?err, "Failed to retrieve wifi status");
                    true
                }
            };

            display_controller.set_is_wifi_offline(is_offline);
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
        let state = manager.device_state().await;

        match state {
            crate::manager::BmcState::FactoryDefault => {
                let ssid = manager.wifi_ssid();
                display_controller.set_wifi_ssid(ssid);
                display_controller.set_screen(Screen::InitialSetupStart);

                // NOTE: Remove upgrade flag when device hasn't been set up yet
                manager.check_and_remove_upgrade_marker().await;
            }
            crate::manager::BmcState::SetupPending => {
                let ip = Self::show_wifi_connect_screen(&display_controller, manager.clone()).await;

                if ip.is_some() {
                    display_controller.set_connect_ip_qr_code(ip);
                    display_controller.set_screen(Screen::InitialSetupConnectInfo);
                } else {
                    display_controller.set_screen(Screen::InitialSetupGeneralError);
                    _ = manager.factory_reset(false).await;
                }
            }
            crate::manager::BmcState::Operational => {
                if manager.check_and_remove_upgrade_marker().await {
                    display_controller.set_screen(Screen::UpgradeSuccess);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }

                let ip = Self::show_wifi_connect_screen(&display_controller, manager.clone()).await;

                let duration = if ip.is_some() {
                    display_controller.set_connect_ip_qr_code(ip);
                    display_controller.set_screen(Screen::ConnectInfo);
                    CONNECT_INFO_SCREEN_DURATION
                } else {
                    display_controller.set_screen(Screen::WifiConnectFailed);
                    ERROR_SCREEN_DURATION
                };

                tokio::time::sleep(duration).await;
                display_controller.set_screen(Screen::Void);
            }
        }
    }

    async fn show_wifi_connect_screen(
        display_controller: &DisplayController,
        manager: Arc<T>,
    ) -> Option<IpAddr> {
        display_controller.set_screen(Screen::WifiConnectProgress);

        let ssid = manager
            .wifi_saved_networks()
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|wifi| wifi.enabled)
            .and_then(|wifi| wifi.configuration)
            .map(|config| config.ssid);

        if let Some(ssid) = ssid {
            display_controller.set_wifi_ssid(ssid);
        }

        let mut i = 0;
        loop {
            let ip = manager.ip_address().await;

            if ip.is_some() || i >= CHECK_IP_ATTEMPTS {
                return ip;
            }

            tokio::time::sleep(CHECK_IP_WAIT_DURATION).await;
            i += 1;
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

    async fn run_alarm_event_listener(display_controller: DisplayController, alarm_bus: AlarmBus) {
        let mut alarm_receiver = display_controller.on_alarm_events();
        while let Some(event) = alarm_receiver.next().await {
            debug!("Alarm event received [{:?}], sending to AlarmBus", event);
            match event {
                bmc_display::display_controller::callback::AlarmEvent::Stop => alarm_bus.stop_all(),
                bmc_display::display_controller::callback::AlarmEvent::Snooze => alarm_bus.snooze(),
            }
        }
    }
}
