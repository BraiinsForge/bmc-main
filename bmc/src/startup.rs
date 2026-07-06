// Copyright (C) 2025  Braiins Systems s.r.o.

use std::fmt::Debug;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::alarm::{AlarmBus, AlarmController};
use crate::backlight::DisplayBacklightDriver;
use crate::button_manager::ButtonManager;
use crate::compositor::{Compositor, CompositorEvent};
use crate::config::ConfigHandle;
use crate::initial_setup::InitialSetup;
use crate::led::{LedController, run_led_state_task};
use crate::led_coordinator::LedCoordinatorHandle;
use crate::manager::BmcManager;
use crate::sound::SoundController;
use crate::system_manager::SystemManager;
use crate::system_upgrade::{StateService, SystemUpgradeService};
use crate::web::{ServerConfig, WebService};
use crate::widget::{Coordinator, UpgradeWidgetLifecycle, WidgetManager, WidgetRegistry};
use anyhow::Result;
use bmc_button::Buttons;
use bmc_led::led_driver::LedDriver;
use bmc_platform::HardwareCapabilities;
use bmc_scheduler::JobScheduler;
use bmc_upgrade::firmware::FirmwareIndex;
use bmc_upgrade::packages::PackageBackend;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, broadcast};
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
    led_coordinator: LedCoordinatorHandle,
    widget_coordinator: Arc<Coordinator>,
    widget_registry: Arc<WidgetRegistry>,
    system_manager: SystemManager<U>,
    sound_controller: SoundController,
    alarm_controller: AlarmController,
    hardware_capabilities: HardwareCapabilities,
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
    #[expect(
        clippy::too_many_lines,
        reason = "init wires all subsystems sequentially; splitting into helpers \
                  would obscure the initialization order without reducing complexity"
    )]
    pub async fn init(
        config: Configuration,
        manager: Arc<T>,
        session_manager: T::SessionManager,
        backlight_driver: Arc<Mutex<U>>,
        led_driver: LedDriver,
        firmware_index: V,
        package_backend: Arc<dyn PackageBackend>,
        buttons: Arc<Box<dyn Buttons + Send + Sync>>,
        compositor: Arc<dyn Compositor>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(config.address).await?;

        let hardware_capabilities = compositor.hardware_capabilities();

        let state_service = StateService::new();

        let config_handle = ConfigHandle::init(
            config.config_path.clone(),
            config.default_brightness_pct,
            config.default_night_mode_brightness_pct,
            config.default_volume_pct,
            config.default_night_mode_volume_pct,
            manager.platform().product(),
        )
        .await;

        let autoupgrade_config = config_handle.autoupgrade();
        let led_enabled = config_handle.led_enabled();

        let config_handle = Arc::new(RwLock::new(config_handle));

        let scheduler = JobScheduler::init(
            manager.watch_timezone_updates(),
            config.crontab_path.clone(),
        )
        .await;

        let widget_manager =
            WidgetManager::init(config.widgets_paths.clone(), config.capture_widget_output).await;
        let widget_registry = widget_manager.registry();
        let widget_coordinator = Arc::new(Coordinator::new(
            widget_manager,
            compositor.clone(),
            widget_registry.clone(),
            hardware_capabilities,
        ));

        let system_upgrade_service = SystemUpgradeService::new(
            firmware_index,
            &config.upgrade_image_path,
            manager.clone(),
            state_service.clone(),
            scheduler.clone(),
            package_backend,
            Arc::new(UpgradeWidgetLifecycle::new(
                widget_coordinator.clone(),
                config_handle.clone(),
            )),
            config.pending_install_path.clone(),
        );

        system_upgrade_service
            .autoupgrade_init(autoupgrade_config)
            .await;

        let sound_controller =
            SoundController::new(config_handle.clone(), config.sounds_dir.clone());

        let initial_setup = InitialSetup::new(
            manager.clone(),
            Arc::new(AtomicBool::new(false)),
            config_handle.clone(),
            system_upgrade_service.clone(),
        );

        let alarm_bus = AlarmBus::new();

        let alarm_controller = AlarmController::init(
            config_handle.clone(),
            scheduler.clone(),
            sound_controller.clone(),
            alarm_bus.clone(),
            manager.watch_timezone_updates(),
        )
        .await;

        let (_, last_price_change_24h_receiver) = watch::channel(0.0);
        let (mut led_controller, led_state_sender, led_state_receiver) = LedController::new(
            &state_service,
            manager.clone(),
            last_price_change_24h_receiver,
            led_enabled,
            alarm_bus,
        );

        let led_coordinator =
            crate::led_coordinator::spawn_led_coordinator(led_driver.command_sender.clone());

        tokio::spawn(run_led_state_task(
            led_state_receiver,
            led_coordinator.clone(),
        ));

        led_controller.init(led_driver.command_sender.clone(), led_coordinator.clone());
        led_controller.push_event(bmc_led::data::LedEvent::DeviceReady);

        let screen_activity = Arc::new(tokio::sync::Notify::new());
        let button_manager = ButtonManager::new(buttons, manager.clone(), screen_activity.clone());
        let compositor_for_events = compositor.clone();

        let system_manager = SystemManager::init(
            config_handle.clone(),
            manager.watch_timezone_updates(),
            backlight_driver,
            scheduler.clone(),
            sound_controller.clone(),
            led_state_sender,
            manager.clone(),
            screen_activity,
        )
        .await;
        let mut screen_woken_rx = system_manager.subscribe_screen_woken();

        let screen_activity_for_touch = button_manager.screen_activity.clone();
        tokio::spawn(async move {
            let mut event_rx = compositor_for_events.subscribe_events();
            loop {
                match event_rx.recv().await {
                    Ok(CompositorEvent::ScreenActivity) => {
                        screen_activity_for_touch.notify_waiters();
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "compositor event receiver lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let compositor_for_wake = compositor.clone();
        tokio::spawn(async move {
            loop {
                match screen_woken_rx.recv().await {
                    Ok(()) => {
                        if let Err(err) = compositor_for_wake.reset_scene_cycle() {
                            tracing::warn!(error = %err, "Failed to reset scene cycle on wake");
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "screen_woken receiver lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        {
            let config_guard = config_handle.read().await;
            let localization_rx = config_guard.subscribe_localization_change();
            drop(config_guard);
            Coordinator::start_settings_listener(
                widget_coordinator.compositor(),
                localization_rx,
                system_manager.subscribe_night_mode(),
                manager.watch_timezone_updates(),
                alarm_controller.subscribe_next_alarm(),
            );
        }

        let (initial_widget_scene_map, scenes_rx) = {
            let config_guard = config_handle.read().await;
            (
                config_guard.widget_scene_map(),
                config_guard.subscribe_scenes_change(),
            )
        };

        crate::widget::action_handler::spawn_action_handler(
            crate::widget::action_handler::CompositorIo {
                action_rx: compositor.action_receiver(),
                settings_rx: compositor.settings_receiver(),
                active_scene_rx: compositor.active_scene_watch(),
                connected_widgets_rx: compositor.connected_widgets_watch(),
                status_tx: compositor.request_status_sender(),
                night_mode_active_rx: system_manager.subscribe_night_mode(),
                upgrade_state_rx: state_service.subscribe(),
            },
            sound_controller.clone(),
            led_coordinator.clone(),
            initial_widget_scene_map,
            scenes_rx,
            system_manager.clone(),
            manager.clone(),
            compositor.clone(),
        );

        {
            let config_guard = config_handle.read().await;
            let localization = config_guard.localization_config();
            let timezone = manager.timezone();
            let night_mode_active = *system_manager.subscribe_night_mode().borrow();
            let next_alarm = alarm_controller.subscribe_next_alarm().borrow().clone();
            let scene_cycling = config_guard.scene_cycling();
            if let Err(err) = widget_coordinator
                .compositor()
                .set_scene_cycling_config(scene_cycling)
            {
                tracing::warn!(error = %err, "failed to configure scene cycling");
            }
            widget_coordinator
                .spawn_initial_widgets(
                    config_guard.scenes(),
                    &localization,
                    &timezone,
                    night_mode_active,
                    next_alarm,
                )
                .await;
        }

        crate::widget::coordinator::start_brightness_listener(
            compositor.clone(),
            config_handle.clone(),
            config_handle
                .read()
                .await
                .subscribe_brightness_settings_change(),
            system_manager.subscribe_night_mode(),
        );
        crate::widget::coordinator::start_wifi_reconfig_listener(
            compositor.clone(),
            manager.clone(),
            manager.watch_wifi_reconfig(),
        );

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
            led_coordinator,
            widget_coordinator,
            widget_registry,
            system_manager,
            sound_controller,
            alarm_controller,
            hardware_capabilities,
        })
    }

    pub async fn run(self) -> Result<()> {
        let address = self.listener.local_addr()?;
        info!("Starting server on http://{}", address);

        tokio::spawn(self.button_manager.run());

        WebService::new(
            self.manager,
            self.session_manager,
            self.config.server_config,
            self.system_upgrade_service,
            self.config_handle,
            self.initial_setup,
            self.led_controller,
            self.widget_registry,
            self.widget_coordinator.clone(),
            self.led_coordinator,
            self.system_manager,
            self.sound_controller,
            self.alarm_controller,
            self.hardware_capabilities,
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
    pub capture_widget_output: bool,
    pub nix_servers_config_path: PathBuf,
    pub nix_profile_dir: PathBuf,
    pub pending_install_path: PathBuf,
    pub nix_hooks_dir: String,
    pub nix_hooks_override_path: Option<PathBuf>,
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
    const NIX_SERVERS_CONFIG_PATH: &str = "/etc/nix-upgrade/servers.json";
    const NIX_PROFILE_DIR: &str = "/nix/var/nix/gcroots/profiles/bmc";
    // Handoff for a firmware-carried widget install: bmc writes it here, the
    // sysupgrade sequence consumes it via `bmc-nix-cli upgrade --install-from`
    // before the flash in the same boot, so it never crosses the reboot and its
    // tmpfs location is fine. Wiring that sysupgrade step on-device is a
    // cross-repo follow-up; see docs/devel/firmware-package-interlinking.md.
    const PENDING_INSTALL_PATH: &str = "/tmp/bmc-nix-pending-install.json";
    const NIX_HOOKS_DIR: &str = "hooks";
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
            capture_widget_output: false,
            nix_servers_config_path: PathBuf::from(Self::NIX_SERVERS_CONFIG_PATH),
            nix_profile_dir: PathBuf::from(Self::NIX_PROFILE_DIR),
            pending_install_path: PathBuf::from(Self::PENDING_INSTALL_PATH),
            nix_hooks_dir: Self::NIX_HOOKS_DIR.to_owned(),
            nix_hooks_override_path: None,
        }
    }
}
