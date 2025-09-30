// Copyright (C) 2025  Braiins Systems s.r.o.

use std::fmt::Debug;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::alarm::{AlarmBus, AlarmController};
use crate::button_manager::ButtonManager;
use crate::config::ConfigHandle;
use crate::display_tasks::DisplayTasks;
use crate::initial_setup::InitialSetup;
use crate::led::{LedController, LedState};
use crate::manager::BmcManager;
use crate::sound::SoundController;
use crate::system_manager::SystemManager;
use crate::system_upgrade::{StateService, SystemUpgradeService};
use crate::web::{ServerConfig, WebService};
use crate::widget_tasks::WidgetTasks;
use anyhow::Result;
use bmc_button::Buttons;
use bmc_display::display_controller::DisplayController;
use bmc_display::display_driver::{DisplayBacklightDriver, DisplayDriver};
use bmc_led::led_driver::LedDriver;
use bmc_scheduler::JobScheduler;
use bmc_upgrade::firmware::{FirmwareIndex, FirmwareResolver};
use tokio::net::TcpListener;
use tokio::sync::{RwLock, watch};
use tracing::info;

#[derive(Debug)]
pub struct App<T, U, V>
where
    T: BmcManager,
    U: DisplayBacklightDriver,
    V: FirmwareIndex,
{
    listener: TcpListener,
    manager: Arc<T>,
    session_manager: Arc<T::SessionManager>,
    config: Configuration,
    display_tasks: DisplayTasks<T>,
    widget_tasks: WidgetTasks,
    system_upgrade_service: SystemUpgradeService<V, T>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    display_controller: DisplayController,
    initial_setup: InitialSetup<T, V>,
    system_manager: SystemManager<U>,
    sound_controller: SoundController,
    alarm_controller: AlarmController,
    button_manager: ButtonManager<T>,
    led_state_sender: watch::Sender<LedState>,
}

impl<T, U, V> App<T, U, V>
where
    T: BmcManager,
    U: DisplayBacklightDriver,
    V: FirmwareIndex,
{
    #[expect(clippy::too_many_lines)]
    pub async fn init(
        config: Configuration,
        manager: Arc<T>,
        session_manager: T::SessionManager,
        display_driver: DisplayDriver<U>,
        led_driver: LedDriver,
        firmware_resolver: FirmwareResolver<V>,
        buttons: Arc<Box<dyn Buttons + Send + Sync>>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(config.address).await?;

        let state_service = StateService::new();

        let config_handle = ConfigHandle::init(
            config.config_path.clone(),
            config.default_brightness_pct,
            config.default_night_mode_brightness_pct,
            config.default_volume_pct,
            config.default_night_mode_volume_pct,
        )
        .await;

        let display_controller = display_driver.display_controller.clone();
        display_controller.set_scenes(config_handle.scenes.clone());
        display_controller.set_scene_cycling(config_handle.scene_cycling());

        let autoupgrade_config = config_handle.autoupgrade();
        let led_enabled = config_handle.led_enabled();

        let config_handle = Arc::new(RwLock::new(config_handle));

        let scheduler = JobScheduler::init(
            manager.watch_timezone_updates(),
            config.crontab_path.clone(),
        )
        .await;

        let system_upgrade_service = SystemUpgradeService::new(
            firmware_resolver,
            &config.upgrade_image_path,
            manager.clone(),
            state_service.clone(),
            scheduler.clone(),
        );

        system_upgrade_service
            .autoupgrade_init(autoupgrade_config)
            .await?;

        let widget_tasks = WidgetTasks::new(
            display_controller.clone(),
            config_handle.clone(),
            manager.watch_timezone_updates(),
        );

        for scene in config_handle.read().await.scenes.values() {
            if scene.enabled {
                widget_tasks
                    .spawn_all(&scene.id, scene.widgets.values())
                    .await;
            }
        }

        let initial_setup = InitialSetup::new(
            manager.clone(),
            Arc::new(AtomicBool::new(false)),
            config_handle.clone(),
            system_upgrade_service.clone(),
        );

        let sound_controller =
            SoundController::new(config_handle.clone(), config.sounds_dir.clone());

        let alarm_bus = AlarmBus::new();

        let alarm_controller = AlarmController::init(
            config_handle.clone(),
            scheduler.clone(),
            sound_controller.clone(),
            display_controller.clone(),
            alarm_bus.clone(),
            manager.watch_timezone_updates(),
        )
        .await?;

        let system_manager = SystemManager::init(
            config_handle.clone(),
            manager.watch_timezone_updates(),
            display_driver.backlight_driver,
            scheduler,
            display_controller.clone(),
            sound_controller.clone(),
        )
        .await?;

        let display_tasks = DisplayTasks::new(
            display_controller.clone(),
            state_service.subscribe(),
            manager.watch_timezone_updates(),
            initial_setup.subscribe(),
            manager.clone(),
            config_handle.clone(),
            alarm_bus.clone(),
        );

        let (_, last_price_change_24h_receiver) = watch::channel(0.0);
        let (mut led_controller, led_state_sender) = LedController::new(
            &state_service,
            manager.clone(),
            last_price_change_24h_receiver,
            led_enabled,
            alarm_bus,
        );

        led_controller.init(led_driver.command_sender.clone());
        led_controller.push_event(bmc_led::data::LedEvent::DeviceReady);

        let button_manager = ButtonManager::new(buttons, manager.clone());

        Ok(Self {
            listener,
            manager,
            session_manager: session_manager.into(),
            config,
            display_tasks,
            widget_tasks,
            system_upgrade_service,
            config_handle,
            display_controller,
            initial_setup,
            system_manager,
            sound_controller,
            alarm_controller,
            button_manager,
            led_state_sender,
        })
    }

    pub async fn run(self) -> Result<()> {
        let address = self.listener.local_addr()?;
        info!("Starting server on http://{}", address);

        self.display_tasks.spawn();

        tokio::spawn(self.button_manager.run());

        WebService::new(
            self.manager,
            self.session_manager,
            self.config.server_config,
            self.system_upgrade_service,
            self.config_handle,
            self.display_controller,
            self.widget_tasks,
            self.initial_setup,
            self.system_manager,
            self.sound_controller,
            self.alarm_controller,
            self.led_state_sender,
        )
        .run(self.listener)
        .await?;

        Ok(())
    }

    pub fn port(&self) -> Result<u16> {
        Ok(self.listener.local_addr()?.port())
    }
}

#[derive(Debug)]
pub struct Configuration {
    pub address: SocketAddr,
    pub server_config: ServerConfig,
    pub upgrade_image_path: PathBuf,
    pub config_path: PathBuf,
    pub default_brightness_pct: u8,
    pub default_night_mode_brightness_pct: u8,
    pub default_volume_pct: u8,
    pub default_night_mode_volume_pct: u8,
    pub sounds_dir: PathBuf,
    pub crontab_path: Option<PathBuf>,
}

impl Configuration {
    const UPGRADE_IMAGE_PATH: &'static str = "/tmp/firmware.tar";
    const CONFIG_PATH: &'static str = "/etc/bmc_config.json";
    const DEFAULT_BRIGHTNESS_PCT: u8 = 60;
    const DEFAULT_NIGHT_MODE_BRIGHTNESS_PCT: u8 = 25;
    const DEFAULT_VOLUME_PCT: u8 = 60;
    const DEFAULT_NIGHT_MODE_VOLUME_PCT: u8 = 40;
    const SOUNDS_DIR: &str = "/usr/share/bmc/sounds/";
    const CRONTAB_PATH: &str = "/etc/crontabs/root";
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 80),
            server_config: ServerConfig::default(),
            upgrade_image_path: PathBuf::from(Self::UPGRADE_IMAGE_PATH),
            config_path: Self::CONFIG_PATH.into(),
            default_brightness_pct: Self::DEFAULT_BRIGHTNESS_PCT,
            default_night_mode_brightness_pct: Self::DEFAULT_NIGHT_MODE_BRIGHTNESS_PCT,
            default_volume_pct: Self::DEFAULT_VOLUME_PCT,
            default_night_mode_volume_pct: Self::DEFAULT_NIGHT_MODE_VOLUME_PCT,
            sounds_dir: PathBuf::from(Self::SOUNDS_DIR),
            crontab_path: Some(Self::CRONTAB_PATH.into()),
        }
    }
}
