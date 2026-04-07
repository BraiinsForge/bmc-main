// Copyright (C) 2025  Braiins Systems s.r.o.

use std::fmt::Debug;
use std::marker::PhantomData;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::alarm::AlarmBus;
use crate::backlight::DisplayBacklightDriver;
use crate::button_manager::ButtonManager;
use crate::compositor::Compositor;
use crate::config::ConfigHandle;
// TODO: display refactor — re-enable once a replacement display layer ships
// use crate::display_tasks::DisplayTasks;
use crate::initial_setup::InitialSetup;
use crate::led::LedController;
use crate::manager::BmcManager;
use crate::system_upgrade::{StateService, SystemUpgradeService};
use crate::web::{ServerConfig, WebService};
use crate::widget::{Coordinator, WidgetManager, WidgetRegistry};
// TODO: display refactor
// use crate::widget_tasks::WidgetTasks;
use anyhow::Result;
use bmc_button::Buttons;
use bmc_led::led_driver::LedDriver;
use bmc_scheduler::JobScheduler;
use bmc_upgrade::firmware::FirmwareIndex;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
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
    system_upgrade_service: SystemUpgradeService<V, T>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    initial_setup: InitialSetup<T, V>,
    button_manager: ButtonManager<T>,
    led_controller: LedController<T>,
    widget_coordinator: Arc<Coordinator>,
    widget_registry: Arc<WidgetRegistry>,
    _backlight_driver: PhantomData<U>,
}

impl<T, U, V> App<T, U, V>
where
    T: BmcManager,
    U: DisplayBacklightDriver,
    V: FirmwareIndex,
{
    #[expect(
        clippy::too_many_arguments,
        reason = "initialization collects independent subsystem handles; grouping them \
                  into a wrapper struct just to satisfy the lint would hurt clarity"
    )]
    pub async fn init(
        config: Configuration,
        manager: Arc<T>,
        session_manager: T::SessionManager,
        _backlight_driver: Arc<Mutex<U>>,
        led_driver: LedDriver,
        firmware_index: V,
        buttons: Arc<Box<dyn Buttons + Send + Sync>>,
        compositor: Arc<dyn Compositor>,
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

        // TODO: display refactor
        // let display_controller = display_driver.display_controller.clone();
        // display_controller.set_scenes(config_handle.scenes.clone());
        // display_controller.set_scene_cycling(config_handle.scene_cycling());

        let autoupgrade_config = config_handle.autoupgrade();
        let led_enabled = config_handle.led_enabled();

        let config_handle = Arc::new(RwLock::new(config_handle));

        let scheduler = JobScheduler::init(
            manager.watch_timezone_updates(),
            config.crontab_path.clone(),
        )
        .await;

        let system_upgrade_service = SystemUpgradeService::new(
            firmware_index,
            &config.upgrade_image_path,
            manager.clone(),
            state_service.clone(),
            scheduler.clone(),
        );

        system_upgrade_service
            .autoupgrade_init(autoupgrade_config)
            .await;

        // TODO: display refactor - connected_widgets needs new implementation
        // let sound_controller =
        //     SoundController::new(config_handle.clone(), config.sounds_dir.clone());

        // let widget_tasks = WidgetTasks::new(
        //     display_controller.clone(),
        //     config_handle.clone(),
        //     manager.watch_timezone_updates(),
        // );

        // for scene in config_handle.read().await.scenes.values() {
        //     if scene.enabled {
        //         widget_tasks
        //             .spawn_all(&scene.id, scene.widgets.values())
        //             .await;
        //     }
        // }

        let initial_setup = InitialSetup::new(
            manager.clone(),
            Arc::new(AtomicBool::new(false)),
            config_handle.clone(),
            system_upgrade_service.clone(),
        );

        let alarm_bus = AlarmBus::new();

        // TODO: display refactor - AlarmController needs display_controller
        // let alarm_controller = AlarmController::init(
        //     config_handle.clone(),
        //     scheduler.clone(),
        //     sound_controller.clone(),
        //     display_controller.clone(),
        //     alarm_bus.clone(),
        //     manager.watch_timezone_updates(),
        // )
        // .await;

        // TODO: display refactor - SystemManager needs display_controller
        // let system_manager = SystemManager::init(
        //     config_handle.clone(),
        //     manager.watch_timezone_updates(),
        //     backlight_driver,
        //     scheduler,
        //     display_controller.clone(),
        //     sound_controller.clone(),
        // )
        // .await;

        // TODO: display refactor
        // let display_tasks = DisplayTasks::new(
        //     display_controller.clone(),
        //     state_service.subscribe(),
        //     manager.watch_timezone_updates(),
        //     initial_setup.subscribe(),
        //     manager.clone(),
        //     config_handle.clone(),
        //     alarm_bus.clone(),
        //     system_manager.clone(),
        // );

        let (_, last_price_change_24h_receiver) = watch::channel(0.0);
        let (mut led_controller, _led_state_sender) = LedController::new(
            &state_service,
            manager.clone(),
            last_price_change_24h_receiver,
            led_enabled,
            alarm_bus,
        );

        led_controller.init(led_driver.command_sender.clone());
        led_controller.push_event(bmc_led::data::LedEvent::DeviceReady);

        let button_manager = ButtonManager::new(buttons, manager.clone());

        let widget_manager = WidgetManager::init(config.widgets_paths.clone()).await;
        let widget_registry = widget_manager.registry();
        let widget_coordinator = Arc::new(Coordinator::new(widget_manager, compositor));

        {
            let config_guard = config_handle.read().await;
            let localization = config_guard.localization_config();
            let timezone = manager.timezone();
            let night_mode_active = config_guard.night_mode().enabled;
            widget_coordinator
                .spawn_initial_widgets(
                    &config_guard.scenes,
                    &localization,
                    &timezone,
                    night_mode_active,
                )
                .await;
        }

        Ok(Self {
            listener,
            manager,
            session_manager: session_manager.into(),
            config,
            system_upgrade_service,
            config_handle,
            initial_setup,
            button_manager,
            led_controller,
            widget_coordinator,
            widget_registry,
            _backlight_driver: PhantomData,
        })
    }

    pub async fn run(self) -> Result<()> {
        let address = self.listener.local_addr()?;
        info!("Starting server on http://{}", address);

        tokio::spawn(self.button_manager.run());

        WebService::<T, _, V, U>::new(
            self.manager,
            self.session_manager,
            self.config.server_config,
            self.system_upgrade_service,
            self.config_handle,
            self.initial_setup,
            self.led_controller,
            self.widget_registry,
            self.widget_coordinator.clone(),
            // TODO: display refactor — re-enable display-dependent services here.
            // self.system_manager,
            // self.sound_controller,
            // self.alarm_controller,
        )
        .run(self.listener)
        .await?;

        // In case the app panics, this is not executed.
        // The children are SIGKILL'd thanks to kill_on_drop(true).
        self.widget_coordinator.stop_all().await;
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
    pub widgets_paths: Vec<PathBuf>,
}

impl Configuration {
    const UPGRADE_IMAGE_PATH: &str = "/tmp/firmware.tar";
    const CONFIG_PATH: &str = "/etc/bmc_config.json";
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
            address: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 80),
            server_config: ServerConfig::default(),
            upgrade_image_path: PathBuf::from(Self::UPGRADE_IMAGE_PATH),
            config_path: Self::CONFIG_PATH.into(),
            default_brightness_pct: Self::DEFAULT_BRIGHTNESS_PCT,
            default_night_mode_brightness_pct: Self::DEFAULT_NIGHT_MODE_BRIGHTNESS_PCT,
            default_volume_pct: Self::DEFAULT_VOLUME_PCT,
            default_night_mode_volume_pct: Self::DEFAULT_NIGHT_MODE_VOLUME_PCT,
            sounds_dir: PathBuf::from(Self::SOUNDS_DIR),
            crontab_path: Some(Self::CRONTAB_PATH.into()),
            widgets_paths: vec![],
        }
    }
}
