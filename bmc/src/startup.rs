// Copyright (C) 2025  Braiins Systems s.r.o.

use std::fmt::Debug;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::alarm::{AlarmBus, AlarmController};
use crate::config::ConfigHandle;
use crate::display_tasks::DisplayTasks;
use crate::initial_setup::InitialSetup;
use crate::led::LedController;
use crate::sound::SoundController;
use crate::system_manager::SystemManager;
use crate::system_upgrade::{StateService, SystemUpgradeService};
use anyhow::Result;
use bmc_display::display_controller::DisplayController;
use bmc_display::display_driver::{DisplayBacklightDriver, DisplayDriver};
use bmc_led::led_driver::LedDriver;
use bmc_scheduler::JobScheduler;
use bmc_upgrade::firmware::{FirmwareIndex, FirmwareResolver};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::info;

use crate::manager::BmcManager;
use crate::web::{ServerConfig, WebService};
use crate::widget_tasks::WidgetTasks;

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
    _led_driver: LedDriver,
    _led_controller: LedController,
    system_upgrade_service: SystemUpgradeService<V, T>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    display_controller: DisplayController,
    initial_setup: InitialSetup<T>,
    system_manager: SystemManager<U>,
    sound_controller: SoundController,
    alarm_controller: AlarmController,
}

impl<T, U, V> App<T, U, V>
where
    T: BmcManager,
    U: DisplayBacklightDriver,
    V: FirmwareIndex,
{
    pub async fn init(
        config: Configuration,
        manager: Arc<T>,
        session_manager: T::SessionManager,
        display_driver: DisplayDriver<U>,
        led_driver: LedDriver,
        firmware_resolver: FirmwareResolver<V>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(config.address).await?;

        let state_service = StateService::new();

        let system_upgrade_service = SystemUpgradeService::new(
            firmware_resolver,
            &config.upgrade_image_path,
            manager.clone(),
            state_service.clone(),
        );

        let config_handle = ConfigHandle::init(
            config.config_path.clone(),
            config.default_brightness_pct,
            config.default_night_mode_brightness_pct,
            config.default_volume_pct,
            config.default_night_mode_volume_pct,
        )
        .await?;

        let display_controller = display_driver.display_controller.clone();
        display_controller.set_scenes(config_handle.scenes.clone());
        display_controller.set_scene_cycling(config_handle.scene_cycling());

        let config_handle = Arc::new(RwLock::new(config_handle));

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
        );

        let scheduler = JobScheduler::init(manager.watch_timezone_updates(), None).await;

        let sound_controller =
            SoundController::new(config_handle.clone(), config.sounds_dir.clone());

        let alarm_bus = AlarmBus::new();

        let alarm_controller = AlarmController::init(
            config_handle.clone(),
            scheduler.clone(),
            sound_controller.clone(),
            display_controller.clone(),
            alarm_bus.clone(),
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
            alarm_bus,
        );

        let led_controller = LedController::new(&state_service);
        led_controller.init(led_driver.command_sender.clone());

        Ok(Self {
            listener,
            manager,
            session_manager: session_manager.into(),
            config,
            display_tasks,
            widget_tasks,
            _led_driver: led_driver,
            _led_controller: led_controller,
            system_upgrade_service,
            config_handle,
            display_controller,
            initial_setup,
            system_manager,
            sound_controller,
            alarm_controller,
        })
    }

    pub async fn run(self) -> Result<()> {
        let address = self.listener.local_addr()?;
        info!("Starting server on http://{}", address);

        self.display_tasks.spawn();

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
}

impl Configuration {
    const UPGRADE_IMAGE_PATH: &'static str = "/tmp/firmware.tar";
    const CONFIG_PATH: &'static str = "/etc/bmc_config.json";
    const DEFAULT_BRIGHTNESS_PCT: u8 = 80;
    const DEFAULT_NIGHT_MODE_BRIGHTNESS_PCT: u8 = 50;
    const DEFAULT_VOLUME_PCT: u8 = 80;
    const DEFAULT_NIGHT_MODE_VOLUME_PCT: u8 = 50;
    const SOUNDS_DIR: &str = "/usr/share/bmc/sounds/";
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
        }
    }
}
