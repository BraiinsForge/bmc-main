// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::BmcManager;
use crate::alarm::{AlarmBus, AlarmEvent};
use crate::initial_setup::InitSetupState;
use crate::system_upgrade::SystemUpgradeState;

use crate::config::ConfigHandle;
use bmc_display::bitcoin_data::BitcoinData;
use bmc_display::blockheight_data::BlockheightData;
use bmc_display::data::{ConnectInfoScreen, InitScreen, UpgradeScreen};
use bmc_display::difficulty_data::DifficultyData;
use bmc_display::display_controller::DisplayController;
use bmc_display::hashrate_data::HashrateData;
use bmc_shared_ii_net::wifi::SignalStrength;
use bmc_shared_time::time::Timezone;
use futures::StreamExt;
use reqwest::Client;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, broadcast, watch};
use tokio::time::interval;
use tracing::{debug, error, info, warn};

const SCREEN_DURATION: Duration = Duration::from_secs(5);

const PRICE_API_URL: &str = "https://public-api.braiins.com/v1/price-stats";
const BLOCK_HEIGHT_API_URL: &str = "https://public-api.braiins.com/v2/blocks";
const BLOCK_HEIGHT_LIMIT_API_PARAM: &str = "limit";
const CURRENCY_API_PARAM: &str = "currency";
const DIFFICULTY_STATS_URL: &str = "https://public-api.braiins.com/v1/difficulty-stats";
const HASHRATE_STATS_URL: &str = "https://public-api.braiins.com/v2/hashrate-stats";
const API_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_INFO_SCREEN_DURATION: Duration = Duration::from_secs(10);
const ERROR_SCREEN_DURATION: Duration = Duration::from_secs(5);
const CHECK_IP_ATTEMPTS: u8 = 10;
const CHECK_IP_WAIT_DURATION: Duration = Duration::from_secs(2);
const DEFAULT_ALARM_LABEL: &str = "Alarm";

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

        tokio::spawn(Self::run_btc_price_update(
            display_controller.clone(),
            config_handle.clone(),
        ));

        tokio::spawn(Self::run_blockheight_update(
            display_controller.clone(),
            config_handle.clone(),
            manager.clone(),
        ));

        tokio::spawn(Self::run_difficulty_stats_update(
            display_controller.clone(),
            config_handle.clone(),
        ));

        tokio::spawn(Self::run_hashrate_stats_update(
            display_controller.clone(),
            config_handle.clone(),
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

        // NOTE: Propagate events from Slint to Alarm controller, e.g. stop/snooze alarm
        tokio::spawn(Self::run_alarm_slint_event_listener(
            display_controller.clone(),
            alarm_bus.clone(),
        ));

        //NOTE: Propagate events from Alarm controller to display, e.g. alarm was triggered, show alarm screen
        tokio::spawn(Self::run_alarm_event_listener(
            display_controller,
            alarm_bus.subscribe_events(),
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
            let Some(upgrade_state) = receiver.borrow_and_update().clone() else {
                continue;
            };

            match upgrade_state {
                SystemUpgradeState::DownloadStarted { total_mb } => {
                    display_controller.update_download_firmware_progress(0.0, total_mb);
                    display_controller.set_upgrade_screen(Some(UpgradeScreen::DownloadFirmware));
                }
                SystemUpgradeState::DownloadProgress {
                    downloaded_mb,
                    total_mb,
                } => {
                    display_controller.update_download_firmware_progress(downloaded_mb, total_mb);
                }
                SystemUpgradeState::DownloadFinished { total_mb, .. } => {
                    display_controller.update_download_firmware_progress(total_mb, total_mb);
                }
                SystemUpgradeState::UpgradeStarted => {
                    display_controller.set_upgrade_screen(Some(UpgradeScreen::Upgrade));
                    // NOTE: upgrade_screen will be turned off in `run_init_display_screen` (operational case)
                }
                SystemUpgradeState::Failed => {
                    display_controller.set_upgrade_screen(Some(UpgradeScreen::UpgradeFailed));
                    tokio::time::sleep(SCREEN_DURATION).await;
                    display_controller.set_upgrade_screen(None);
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
                        display_controller.set_init_screen(Some(InitScreen::SetupWifiConnecting));
                    }
                    InitSetupState::WifiConnectionSuccess => {
                        display_controller.set_init_screen(Some(InitScreen::SetupWifiConnected));
                        tokio::time::sleep(SCREEN_DURATION).await;
                        let ip = manager.ip_address().await;
                        display_controller.set_connect_ip_qr_code(ip);
                        display_controller.set_init_screen(Some(InitScreen::SetupConnectInfo));
                    }
                    InitSetupState::WifiConnectionFailed => {
                        display_controller.set_init_screen(Some(InitScreen::SetupWifiError));
                        tokio::time::sleep(SCREEN_DURATION).await;
                        let ssid = manager.wifi_ssid();
                        display_controller.set_wifi_ssid(ssid);
                        display_controller.set_init_screen(Some(InitScreen::SetupStart));
                    }
                    InitSetupState::UnexpectedError => {
                        display_controller.set_init_screen(Some(InitScreen::SetupGeneralError));
                    }
                    InitSetupState::DeviceSetupSuccess => {
                        display_controller.set_init_screen(Some(InitScreen::SetupCompleted));
                        tokio::time::sleep(SCREEN_DURATION).await;
                        display_controller.set_scene_cycler_screen(true);
                        display_controller.set_init_screen(None);
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
                display_controller.set_init_screen(Some(InitScreen::SetupStart));
                // NOTE: init_screen will be turned off in `run_initial_setup_listener`

                // NOTE: Remove upgrade flag when device hasn't been set up yet
                manager.check_and_remove_upgrade_marker().await;
            }
            crate::manager::BmcState::SetupPending => {
                let ip = Self::show_wifi_connect_screen(&display_controller, manager.clone(), true)
                    .await;

                if ip.is_some() {
                    display_controller.set_connect_ip_qr_code(ip);
                    display_controller.set_init_screen(Some(InitScreen::SetupConnectInfo));
                    // NOTE: init_screen will be turned off in `run_initial_setup_listener`
                } else {
                    display_controller.set_init_screen(Some(InitScreen::SetupGeneralError));
                    _ = manager.factory_reset(false).await;
                }
            }
            crate::manager::BmcState::Operational => {
                if manager.check_and_remove_upgrade_marker().await {
                    display_controller.set_upgrade_screen(Some(UpgradeScreen::UpgradeSuccess));
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    display_controller.set_upgrade_screen(None);
                }

                let ip =
                    Self::show_wifi_connect_screen(&display_controller, manager.clone(), false)
                        .await;

                let duration = if ip.is_some() {
                    display_controller.set_connect_ip_qr_code(ip);
                    display_controller
                        .set_connect_info_screen(Some(ConnectInfoScreen::ConnectInfo));
                    CONNECT_INFO_SCREEN_DURATION
                } else {
                    display_controller
                        .set_connect_info_screen(Some(ConnectInfoScreen::WifiConnectFailed));
                    ERROR_SCREEN_DURATION
                };

                tokio::time::sleep(duration).await;
                display_controller.set_scene_cycler_screen(true);
                display_controller.set_connect_info_screen(None);
            }
        }
    }

    async fn show_wifi_connect_screen(
        display_controller: &DisplayController,
        manager: Arc<T>,
        initial: bool,
    ) -> Option<IpAddr> {
        if initial {
            display_controller.set_init_screen(Some(InitScreen::SetupWifiConnecting));
        } else {
            display_controller
                .set_connect_info_screen(Some(ConnectInfoScreen::WifiConnectProgress));
        }

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

    async fn run_btc_price_update(
        display_controller: DisplayController,
        config_handle: Arc<RwLock<ConfigHandle>>,
    ) {
        let mut interval = interval(Duration::from_secs(60));

        loop {
            interval.tick().await;

            debug!("Getting bitcoin data...");
            let client = Client::new();
            let btc_price_data = match client.get(PRICE_API_URL).timeout(API_TIMEOUT).send().await {
                Ok(response) => response
                    .json::<BitcoinData>()
                    .await
                    .map_err(|e| warn!("Failed to parse bitcoin price JSON: {e}"))
                    .unwrap_or_default(),
                Err(e) => {
                    warn!("Failed to get bitcoin price from API: {e}");
                    BitcoinData::default()
                }
            };

            let number_format = config_handle
                .read()
                .await
                .localization_config()
                .number_format;
            display_controller.update_btc_price(btc_price_data, number_format);
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
            let blockheight_data = match client
                .get(BLOCK_HEIGHT_API_URL)
                .query(&[
                    (BLOCK_HEIGHT_LIMIT_API_PARAM, "1"),
                    (CURRENCY_API_PARAM, "usd"),
                ])
                .timeout(API_TIMEOUT)
                .send()
                .await
            {
                Ok(response) => response
                    .json::<Vec<BlockheightData>>()
                    .await
                    .map_err(|e| warn!("Failed to parse blockheight JSON: {e}"))
                    .unwrap_or_default()
                    .first()
                    .cloned()
                    .unwrap_or_default(),
                Err(e) => {
                    warn!("Failed to get blockheight data from API: {e}");
                    BlockheightData::default()
                }
            };

            let timezone = manager.timezone();

            let is_24_format = config_handle
                .read()
                .await
                .localization_config()
                .time_system
                .is_24();
            let date_format = config_handle.read().await.localization_config().date_format;
            let number_format = config_handle
                .read()
                .await
                .localization_config()
                .number_format;

            display_controller.update_blockheight_data(
                blockheight_data,
                timezone,
                is_24_format,
                date_format,
                number_format,
            );
        }
    }

    async fn run_difficulty_stats_update(
        display_controller: DisplayController,
        config_handle: Arc<RwLock<ConfigHandle>>,
    ) {
        let mut interval = interval(Duration::from_secs(60));
        let Ok(client) = reqwest::ClientBuilder::new().timeout(API_TIMEOUT).build() else {
            error!("HTTP Client init failed");
            return;
        };

        loop {
            interval.tick().await;

            debug!("Getting difficulty data...");
            let difficulty_data = match client.get(DIFFICULTY_STATS_URL).send().await {
                Ok(response) => response
                    .json::<DifficultyData>()
                    .await
                    .map_err(|e| warn!("Failed to parse difficulty JSON: {e}"))
                    .unwrap_or_default(),
                Err(e) => {
                    warn!("Failed to get difficulty data from API: {e}");
                    DifficultyData::default()
                }
            };

            let number_format = config_handle
                .read()
                .await
                .localization_config()
                .number_format;

            display_controller.update_difficulty_data(difficulty_data, number_format);
        }
    }

    async fn run_hashrate_stats_update(
        display_controller: DisplayController,
        config_handle: Arc<RwLock<ConfigHandle>>,
    ) {
        let mut interval = interval(Duration::from_secs(60));
        let Ok(client) = reqwest::ClientBuilder::new().timeout(API_TIMEOUT).build() else {
            error!("HTTP Client init failed");
            return;
        };

        loop {
            interval.tick().await;

            debug!("Getting hashrate data...");
            let hashrate_data = match client
                .get(HASHRATE_STATS_URL)
                .query(&[(CURRENCY_API_PARAM, "usd")])
                .send()
                .await
            {
                Ok(response) => response
                    .json::<HashrateData>()
                    .await
                    .map_err(|e| warn!("Failed to parse hashrate JSON: {e}"))
                    .unwrap_or_default(),
                Err(e) => {
                    warn!("Failed to get hashrate data from API: {e}");
                    HashrateData::default()
                }
            };

            let number_format = config_handle
                .read()
                .await
                .localization_config()
                .number_format;

            display_controller.update_hashrate_data(hashrate_data, number_format);
        }
    }

    async fn run_alarm_slint_event_listener(
        display_controller: DisplayController,
        alarm_bus: AlarmBus,
    ) {
        let mut alarm_receiver = display_controller.on_alarm_events();
        while let Some(event) = alarm_receiver.next().await {
            debug!("Alarm event received [{:?}], sending to AlarmBus", event);
            match event {
                bmc_display::display_controller::callback::AlarmEvent::Stop => alarm_bus.stop_all(),
                bmc_display::display_controller::callback::AlarmEvent::Snooze => alarm_bus.snooze(),
            }
        }
    }

    async fn run_alarm_event_listener(
        display_controller: DisplayController,
        mut events_rx: broadcast::Receiver<AlarmEvent>,
    ) {
        while let Ok(event) = events_rx.recv().await {
            match event {
                AlarmEvent::Stopped { .. } | AlarmEvent::Snoozed => {
                    display_controller.set_clock_alarm_screen(false);
                }
                AlarmEvent::Started { alarm } => {
                    let label = if alarm.data.name.is_empty() {
                        DEFAULT_ALARM_LABEL.to_owned()
                    } else {
                        alarm.data.name
                    };

                    let show_snooze = alarm.data.snooze_options.is_some_and(|options| {
                        options
                            .limit
                            .limit()
                            .is_none_or(|limit| alarm.snooze_count < limit)
                    });

                    display_controller.set_alarm_data(label, show_snooze);
                    display_controller.set_clock_alarm_screen(true);
                }
            }
        }
    }
}
