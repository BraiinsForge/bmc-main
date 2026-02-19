// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::BmcManager;
use crate::alarm::{AlarmBus, AlarmEvent};
use crate::initial_setup::InitSetupState;
use crate::system_manager::SystemManager;
use crate::system_upgrade::SystemUpgradeState;

use crate::config::ConfigHandle;
use bmc_display::bitcoin_data::BitcoinData;
use bmc_display::blockheight_data::{self, BlockheightData};
use bmc_display::data::{ConnectInfoScreen, InitScreen, UpgradeScreen};
use bmc_display::difficulty_data::DifficultyData;
use bmc_display::display_controller::DisplayController;
use bmc_display::display_driver::DisplayBacklightDriver;
use bmc_display::hashrate_data::HashrateData;
use bmc_shared_ii_net::wifi::SignalStrength;
use bmc_shared_time::time::Timezone;
use futures::StreamExt;
use reqwest::Client;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::select;
use tokio::sync::{RwLock, broadcast, watch};
use tokio::time::interval;
use tracing::{debug, error, info, warn};

const SCREEN_DURATION: Duration = Duration::from_secs(5);
const DATA_REFRESH_PERIOD: Duration = Duration::from_secs(60);

const PRICE_API_URL: &str = "https://public-api.braiins.com/v1/price-stats";
const CURRENCY_API_PARAM: &str = "currency";
const DIFFICULTY_STATS_URL: &str = "https://public-api.braiins.com/v1/difficulty-stats";
const HASHRATE_STATS_URL: &str = "https://public-api.braiins.com/v2/hashrate-stats";
const API_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_INFO_SCREEN_DURATION: Duration = Duration::from_secs(10);
const ERROR_SCREEN_DURATION: Duration = Duration::from_secs(5);
const CHECK_IP_ATTEMPTS: u8 = 10;
const CHECK_IP_WAIT_DURATION: Duration = Duration::from_secs(2);
const DEFAULT_ALARM_LABEL: &str = "Alarm";
const WIFI_RECONFIG_TIMEOUT: Duration = Duration::from_secs(8 * 60); // 8 minutes
const REBOOT_SLEEP_DURATION: Duration = Duration::from_secs(10);
const WIFI_INTERFACE_MAX_RETRY: usize = 15;
const WIFI_INTERFACE_RETRY_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub(crate) struct DisplayTasks<T: BmcManager, U: DisplayBacklightDriver> {
    display_controller: DisplayController,
    system_upgrade_receiver: watch::Receiver<Option<SystemUpgradeState>>,
    timezone_receiver: watch::Receiver<Timezone>,
    initial_setup_receiver: watch::Receiver<Option<InitSetupState>>,
    manager: Arc<T>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    alarm_bus: AlarmBus,
    system_manager: SystemManager<U>,
}

impl<T: BmcManager, U: DisplayBacklightDriver> DisplayTasks<T, U> {
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn new(
        display_controller: DisplayController,
        system_upgrade_receiver: watch::Receiver<Option<SystemUpgradeState>>,
        timezone_receiver: watch::Receiver<Timezone>,
        initial_setup_receiver: watch::Receiver<Option<InitSetupState>>,
        manager: Arc<T>,
        config_handle: Arc<RwLock<ConfigHandle>>,
        alarm_bus: AlarmBus,
        system_manager: SystemManager<U>,
    ) -> Self {
        Self {
            display_controller,
            system_upgrade_receiver,
            timezone_receiver,
            initial_setup_receiver,
            manager,
            config_handle,
            alarm_bus,
            system_manager,
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
            system_manager,
        } = self;

        tokio::spawn(Self::run_init_display_screen(
            display_controller.clone(),
            manager.clone(),
        ));

        tokio::spawn(Self::run_system_upgrade_listener(
            display_controller.clone(),
            system_upgrade_receiver,
        ));

        tokio::spawn(Self::run_timezone_listener(timezone_receiver.clone()));

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
            timezone_receiver,
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

        tokio::spawn(Self::run_restart_event_listener(
            display_controller.clone(),
            manager.clone(),
        ));

        // NOTE: WiFi reconfigure buttons - two handlers for different behaviors
        tokio::spawn(Self::run_wifi_reconfig_event_listener(
            display_controller.clone(),
            manager.clone(),
        ));
        tokio::spawn(Self::run_wifi_reconfig_restart_event_listener(
            display_controller.clone(),
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

        //NOTE: Propagate events from Alarm controller to display, e.g. alarm was triggered => show alarm screen
        tokio::spawn(Self::run_alarm_event_listener(
            display_controller.clone(),
            alarm_bus.subscribe_events(),
            system_manager.clone(),
        ));

        // NOTE: Propagate brightness events from Slint to system manager
        tokio::spawn(Self::run_brightness_slint_event_listener(
            display_controller.clone(),
            system_manager.clone(),
        ));

        // NOTE: Propagate sound volume events from Slint to system manager
        tokio::spawn(Self::run_sound_slint_event_listener(
            display_controller.clone(),
            system_manager.clone(),
        ));

        // NOTE: Propagate night mode toggle events from Slint to system manager
        tokio::spawn(Self::run_night_mode_toggle_listener(
            display_controller.clone(),
            system_manager.clone(),
        ));

        // NOTE: Dismiss completed countdown scenes via tap
        tokio::spawn(Self::run_countdown_dismiss_listener(
            display_controller.clone(),
            config_handle.clone(),
        ));

        // NOTE: Watch night mode state changes and update UI
        tokio::spawn(Self::run_night_mode_state_watcher(
            display_controller.clone(),
            system_manager.clone(),
        ));

        // NOTE: Propagate touch-to-wake events from Slint to system manager
        tokio::spawn(Self::run_screen_activity_listener(
            display_controller.clone(),
            system_manager.clone(),
        ));
    }

    async fn display_setup_start(display_controller: &DisplayController, manager: Arc<T>) {
        let result = manager
            .wait_for_wifi_ssid(WIFI_INTERFACE_MAX_RETRY, WIFI_INTERFACE_RETRY_DELAY)
            .await;
        if let Ok(ssid) = result {
            display_controller.set_wifi_ssid(ssid);
            display_controller.set_ap_qr_code();
            display_controller.set_init_screen(Some(InitScreen::SetupStart));
        } else {
            display_controller.set_init_screen(Some(InitScreen::SetupGeneralError));
            tokio::time::sleep(REBOOT_SLEEP_DURATION).await;
            _ = manager.reboot().await;
        }
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

                    display_controller
                        .set_wifi_signal_strength(map_signal_strength(signal_strength));

                    signal_strength == SignalStrength::Offline
                }
                Err(err) => {
                    warn!(?err, "Failed to retrieve wifi status");

                    display_controller
                        .set_wifi_signal_strength(bmc_display::data::SignalStrength::Offline);

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
                        Self::display_setup_start(&display_controller, manager.clone()).await;
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
                    InitSetupState::WifiReconfigSuccess => {
                        // WiFi reconfiguration succeeded - return to normal operation
                        display_controller.set_init_screen(Some(InitScreen::SetupWifiConnected));
                        tokio::time::sleep(SCREEN_DURATION).await;
                        // Update IP address for bottom rollette after WiFi change
                        // Small delay to allow DHCP to assign new IP
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        let ip = manager.ip_address().await;
                        display_controller.set_connect_ip_qr_code(ip);
                        display_controller.set_scene_cycler_screen(true);
                        display_controller.set_init_screen(None);
                    }
                }
            }
        }
    }

    async fn run_init_display_screen(display_controller: DisplayController, manager: Arc<T>) {
        let state = manager.device_state().await;

        // Set factory default flag for UI (controls visibility of WiFi reconfig button)
        display_controller
            .set_is_factory_default(matches!(state, crate::manager::BmcState::FactoryDefault));

        match state {
            crate::manager::BmcState::FactoryDefault => {
                Self::display_setup_start(&display_controller, manager.clone()).await;
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
            crate::manager::BmcState::WifiReconfiguration => {
                // WiFi reconfiguration mode - show setup start screen (AP mode active)
                Self::display_setup_start(&display_controller, manager.clone()).await;
                // NOTE: init_screen will be turned off in `run_initial_setup_listener`
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
        let mut interval = interval(DATA_REFRESH_PERIOD);

        let mut localization_change_listener =
            config_handle.read().await.subscribe_localization_change();

        let mut number_format = config_handle
            .read()
            .await
            .localization_config()
            .number_format;

        let client = Client::new();
        let mut btc_price_data = BitcoinData::default();

        loop {
            select! {
                _ = interval.tick() => {
                    debug!("Fetching Bitcoin price data");
                    btc_price_data = match client.get(PRICE_API_URL).timeout(API_TIMEOUT).send().await {
                        Ok(response) => response
                            .json::<BitcoinData>()
                            .await
                            .inspect_err(
                                |err| error!(error = %err, "Failed to parse Bitcoin price JSON response"),
                            )
                            .unwrap_or_default(),
                        Err(err) => {
                            warn!(error = %err, "Failed to fetch Bitcoin price data from API");
                            BitcoinData::default()
                        }
                    };
                }
                Ok(localization) = localization_change_listener.recv() => {
                    number_format = localization.number_format;
                }
            }

            display_controller.update_btc_price(btc_price_data, number_format);
        }
    }

    async fn run_blockheight_update(
        display_controller: DisplayController,
        config_handle: Arc<RwLock<ConfigHandle>>,
        mut timezone_receiver: watch::Receiver<Timezone>,
    ) {
        let mut interval = interval(DATA_REFRESH_PERIOD);

        let mut localization_change_listener =
            config_handle.read().await.subscribe_localization_change();

        let localization = config_handle.read().await.localization_config();
        let mut is_24_format = localization.time_system.is_24();
        let mut date_format = localization.date_format;
        let mut number_format = localization.number_format;
        let mut timezone = timezone_receiver.borrow_and_update().clone();

        let client = Client::new();
        let mut blockheight_data = BlockheightData::default();

        loop {
            select! {
                _ = interval.tick() => {
                    debug!("Fetching Blockheight data");
                    blockheight_data = match client
                        .get(blockheight_data::BLOCK_HEIGHT_API_URL)
                        .query(&[
                            (blockheight_data::BLOCK_HEIGHT_LIMIT_API_PARAM, "1"),
                            (CURRENCY_API_PARAM, "usd"),
                        ])
                        .timeout(API_TIMEOUT)
                        .send()
                        .await
                    {
                        Ok(response) => response
                            .json::<Vec<BlockheightData>>()
                            .await
                            .inspect_err(
                                |err| error!(error = %err, "Failed to parse block height JSON response"),
                            )
                            .unwrap_or_default()
                            .first()
                            .cloned()
                            .unwrap_or_default(),
                        Err(err) => {
                            warn!(error = %err, "Failed to fetch block height data from API");
                            BlockheightData::default()
                        }
                    };
                }
                Ok(localization) = localization_change_listener.recv() => {
                    number_format = localization.number_format;
                    date_format = localization.date_format;
                    is_24_format = localization.time_system.is_24();
                }
                _ = timezone_receiver.changed() => {
                    timezone = timezone_receiver.borrow_and_update().clone();
                }
            }

            display_controller.update_blockheight_data(
                blockheight_data.clone(),
                timezone.clone(),
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
        let mut interval = interval(DATA_REFRESH_PERIOD);

        let mut localization_change_listener =
            config_handle.read().await.subscribe_localization_change();

        let mut number_format = config_handle
            .read()
            .await
            .localization_config()
            .number_format;

        let Ok(client) = reqwest::ClientBuilder::new().timeout(API_TIMEOUT).build() else {
            error!("HTTP Client init failed");
            return;
        };
        let mut difficulty_data = DifficultyData::default();

        loop {
            select! {
                _ = interval.tick() => {
                    debug!("Fetching difficulty data");
                    difficulty_data = match client.get(DIFFICULTY_STATS_URL).send().await {
                        Ok(response) => response
                            .json::<DifficultyData>()
                            .await
                            .inspect_err(
                                |err| error!(error = %err, "Failed to parse difficulty JSON response"),
                            )
                            .unwrap_or_default(),
                        Err(err) => {
                            warn!(error = %err, "Failed to fetch difficulty data from API");
                            DifficultyData::default()
                        }
                    };
                }
                Ok(localization) = localization_change_listener.recv() => {
                    number_format = localization.number_format;
                }
            }

            display_controller.update_difficulty_data(difficulty_data.clone(), number_format);
        }
    }

    async fn run_hashrate_stats_update(
        display_controller: DisplayController,
        config_handle: Arc<RwLock<ConfigHandle>>,
    ) {
        let mut interval = interval(DATA_REFRESH_PERIOD);

        let mut localization_change_listener =
            config_handle.read().await.subscribe_localization_change();

        let mut number_format = config_handle
            .read()
            .await
            .localization_config()
            .number_format;

        let Ok(client) = reqwest::ClientBuilder::new().timeout(API_TIMEOUT).build() else {
            error!("HTTP Client init failed");
            return;
        };
        let mut hashrate_data = HashrateData::default();

        loop {
            select! {
                _ = interval.tick() => {
                    debug!("Fetching hashrate data");
                    hashrate_data = match client
                        .get(HASHRATE_STATS_URL)
                        .query(&[(CURRENCY_API_PARAM, "usd")])
                        .send()
                        .await
                    {
                        Ok(response) => response
                            .json::<HashrateData>()
                            .await
                            .inspect_err(
                                |err| error!(error = %err, "Failed to parse hashrate JSON response"),
                            )
                            .unwrap_or_default(),
                        Err(err) => {
                            warn!(error = %err, "Failed to fetch hashrate data from API");
                            HashrateData::default()
                        }
                    };
                }
                Ok(localization) = localization_change_listener.recv() => {
                    number_format = localization.number_format;
                }
            }

            display_controller.update_hashrate_data(hashrate_data.clone(), number_format);
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

    async fn run_restart_event_listener(display_controller: DisplayController, manager: Arc<T>) {
        let mut restart_receiver = display_controller.on_restart_events();
        while restart_receiver.next().await.is_some() {
            debug!("Restart event received");

            if let Err(e) = manager.reboot().await {
                error!("Failed to reboot: {:?}", e);
            }
        }
    }

    // WiFi failed screen button - guards against spam clicks
    async fn run_wifi_reconfig_event_listener(
        display_controller: DisplayController,
        manager: Arc<T>,
    ) {
        let mut wifi_reconfig_receiver = display_controller.on_wifi_reconfig_events();
        while wifi_reconfig_receiver.next().await.is_some() {
            info!("WiFi reconfigure event received");

            if manager.is_factory_default().await {
                debug!("Device in factory default state, ignoring reconfig click");
                continue;
            }
            if manager.is_wifi_reconfig().await {
                debug!("Already in WiFi reconfiguration mode, ignoring click");
                continue;
            }

            Self::enter_wifi_reconfig_with_display_controller(&display_controller, &manager).await;
        }
    }

    // Bottom rollette button - allows clean restart if AP failed
    async fn run_wifi_reconfig_restart_event_listener(
        display_controller: DisplayController,
        manager: Arc<T>,
    ) {
        let mut wifi_reconfig_receiver = display_controller.on_wifi_reconfig_restart_events();
        while wifi_reconfig_receiver.next().await.is_some() {
            info!("WiFi reconfigure restart event received");

            if manager.is_factory_default().await {
                debug!("Device in factory default state, ignoring reconfig click");
                continue;
            }

            // Exit first for a clean restart
            if manager.is_wifi_reconfig().await {
                info!("Already in WiFi reconfiguration mode, restarting cleanly");
                let _ = manager.exit_wifi_reconfiguration().await;
            }

            Self::enter_wifi_reconfig_with_display_controller(&display_controller, &manager).await;
        }
    }

    async fn enter_wifi_reconfig_with_display_controller(
        display_controller: &DisplayController,
        manager: &Arc<T>,
    ) {
        match manager.enter_wifi_reconfig().await {
            Ok(()) => {
                info!("Entered WiFi reconfiguration mode");
                Self::display_setup_start(display_controller, manager.clone()).await;

                // Spawn timeout task - hides init screen but keeps AP alive
                let manager_clone = manager.clone();
                let display_clone = display_controller.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(WIFI_RECONFIG_TIMEOUT).await;

                    // If still in reconfig mode, show widgets but keep AP running
                    if manager_clone.is_wifi_reconfig().await {
                        info!("WiFi reconfiguration timeout - showing widgets, AP stays active");
                        display_clone.set_init_screen(None);
                        display_clone.set_scene_cycler_screen(true);
                    }
                });
            }
            Err(e) => {
                error!("Failed to enter WiFi reconfiguration: {:?}", e);
            }
        }
    }

    async fn run_alarm_event_listener(
        display_controller: DisplayController,
        mut events_rx: broadcast::Receiver<AlarmEvent>,
        system_manager: SystemManager<U>,
    ) {
        while let Ok(event) = events_rx.recv().await {
            system_manager.notify_screen_activity();
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

    async fn run_brightness_slint_event_listener(
        display_controller: DisplayController,
        system_manager: SystemManager<U>,
    ) {
        let mut brightness_receiver = display_controller.on_brightness_events();
        while let Some(event) = brightness_receiver.next().await {
            debug!("Brightness event received [{:?}]", event);
            system_manager.notify_screen_activity();

            let night_mode_is_active = system_manager.is_night_mode_active();
            let display_settings = system_manager.display_settings().await;

            let current_brightness = if night_mode_is_active {
                display_settings.night_mode_config.brightness_pct
            } else {
                display_settings.brightness_pct
            };

            debug!(
                "Current brightness: {}, night_mode_active: {}",
                current_brightness, night_mode_is_active
            );

            let new_brightness = match event {
                bmc_display::display_controller::callback::BrightnessEvent::Increase => {
                    current_brightness.saturating_add(10).min(100)
                }
                bmc_display::display_controller::callback::BrightnessEvent::Decrease => {
                    current_brightness.saturating_sub(10)
                }
            };

            debug!("New brightness: {}", new_brightness);

            let result = if night_mode_is_active {
                system_manager
                    .set_night_mode_brightness(new_brightness)
                    .await
            } else {
                system_manager.set_brightness(new_brightness).await
            };

            if let Err(e) = result {
                error!("Failed to set brightness: {:?}", e);
            }
        }
    }

    async fn run_sound_slint_event_listener(
        display_controller: DisplayController,
        system_manager: SystemManager<U>,
    ) {
        let mut sound_receiver = display_controller.on_sound_events();
        while let Some(event) = sound_receiver.next().await {
            debug!("Sound event received [{:?}]", event);
            system_manager.notify_screen_activity();

            let night_mode_is_active = system_manager.is_night_mode_active();
            let sound_settings = system_manager.sound_settings().await;

            let current_volume = if night_mode_is_active {
                sound_settings.volume_night_mode
            } else {
                sound_settings.volume
            };

            debug!(
                "Current volume: {}, night_mode_active: {}",
                current_volume, night_mode_is_active
            );

            let new_volume = match event {
                bmc_display::display_controller::callback::SoundEvent::Increase => {
                    current_volume.saturating_add(10).min(100)
                }
                bmc_display::display_controller::callback::SoundEvent::Decrease => {
                    current_volume.saturating_sub(10)
                }
            };

            debug!("New volume: {}", new_volume);

            let result = if night_mode_is_active {
                system_manager.set_sound_volume_night_mode(new_volume).await
            } else {
                system_manager.set_sound_volume(new_volume).await
            };

            if let Err(e) = result {
                error!("Failed to set sound volume: {:?}", e);
            }
        }
    }

    async fn run_countdown_dismiss_listener(
        display_controller: DisplayController,
        config_handle: Arc<RwLock<ConfigHandle>>,
    ) {
        let mut receiver = display_controller.on_countdown_dismiss_events();
        while let Some(event) = receiver.next().await {
            info!(
                "Countdown dismissed: scene={}, widget={}",
                event.scene_id, event.widget_id
            );

            let scene_id: bmc_display::data::SceneId = match event.scene_id.parse() {
                Ok(id) => id,
                Err(_) => continue,
            };

            let cycle_duration = {
                let mut config = config_handle.write().await;
                let mut temp = config.clone();
                let Some(scene) = temp.scenes.get_mut(&scene_id) else {
                    continue;
                };
                if !scene.enabled {
                    continue;
                }

                scene.enabled = false;
                let cycle_duration = scene.cycle_duration;

                if let Err(err) = temp.save().await {
                    error!("Failed to save config after countdown dismiss: {err}");
                    continue;
                }
                *config = temp;
                cycle_duration
            };

            display_controller.update_scene(scene_id, false, cycle_duration);
            display_controller.reset_cycler();
        }
    }

    async fn run_night_mode_toggle_listener(
        display_controller: DisplayController,
        system_manager: SystemManager<U>,
    ) {
        let mut toggle_receiver = display_controller.on_night_mode_toggle_events();
        while toggle_receiver.next().await.is_some() {
            debug!("Night mode toggle event received");
            system_manager.notify_screen_activity();
            if let Err(e) = system_manager.toggle_night_mode().await {
                error!("Failed to toggle night mode: {:?}", e);
            }
        }
    }

    async fn run_screen_activity_listener(
        display_controller: DisplayController,
        system_manager: SystemManager<U>,
    ) {
        let mut activity_receiver = display_controller.on_screen_activity_events();
        while activity_receiver.next().await.is_some() {
            debug!("Screen activity touch event received");
            system_manager.notify_screen_activity();
        }
    }

    async fn run_night_mode_state_watcher(
        display_controller: DisplayController,
        system_manager: SystemManager<U>,
    ) {
        let mut night_mode_receiver = system_manager.subscribe_night_mode();

        loop {
            let is_active = *night_mode_receiver.borrow_and_update();
            let config = system_manager.night_mode_config().await;

            let status_text = if is_active {
                format!("Until {}", config.to.format("%H:%M"))
            } else if config.enabled {
                format!("Until {}", config.from.format("%H:%M"))
            } else {
                String::new()
            };

            display_controller.set_night_mode_ui_state(is_active, status_text);

            if night_mode_receiver.changed().await.is_err() {
                break;
            }
        }
    }
}

fn map_signal_strength(value: SignalStrength) -> bmc_display::data::SignalStrength {
    match value {
        SignalStrength::Offline => bmc_display::data::SignalStrength::Offline,
        SignalStrength::Low => bmc_display::data::SignalStrength::Low,
        SignalStrength::Fair => bmc_display::data::SignalStrength::Fair,
        SignalStrength::Excellent => bmc_display::data::SignalStrength::Strong,
    }
}
