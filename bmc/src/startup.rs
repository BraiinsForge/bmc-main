// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use std::fmt::Debug;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::alarm::{AlarmBus, AlarmController, AlarmEvent};
use crate::backlight::DisplayBacklightDriver;
use crate::button_manager::ButtonManager;
use crate::compositor::{
    AlarmCommand, Compositor, CompositorEvent, UpgradeDisplaySnapshot, UpgradeKind,
    run_night_mode_cycling_task, run_screen_blank_reset_task,
};
use crate::config::ConfigHandle;
use crate::initial_setup::InitialSetup;
use crate::led::{LedController, run_led_state_task};
use crate::led_coordinator::LedCoordinatorHandle;
use crate::manager::{BmcManager, BmcState, UpgradeMarker};
use crate::secret_store::SecretStoreHandle;
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
use tracing::{error, info, warn};

/// Bridge the alarm domain and the alarm overlay through the compositor.
///
/// Outbound: `AlarmBus` events become `deck_alarm_v1` ring/stop signals on the
/// overlay. Inbound: the overlay's dismiss/snooze requests (relayed by the
/// compositor over `alarm_receiver`) become `AlarmBus` commands the
/// `AlarmController` acts on. `AlarmController` itself never learns the
/// compositor exists — this listener is the only translator between them.
fn spawn_alarm_overlay_listener(
    compositor: Arc<dyn Compositor>,
    alarm_bus: AlarmBus,
    config_handle: Arc<RwLock<ConfigHandle>>,
) {
    let mut events = alarm_bus.subscribe_events();
    let mut commands = compositor.alarm_receiver();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                event = events.recv() => match event {
                    Ok(AlarmEvent::Started { alarm }) => {
                        // Render the time in the user's time system; `TimeSystem`
                        // serializes to its strftime pattern (`%I:%M`/`%H:%M`),
                        // same convention as the widget SDK's `format_time`. The
                        // AM/PM marker travels separately so the overlay can
                        // typeset it smaller next to the time.
                        let time_system = config_handle
                            .read()
                            .await
                            .localization_config()
                            .time_system;
                        let time = alarm.data.time.format(&time_system.to_string()).to_string();
                        let period = if time_system.is_24() {
                            String::new()
                        } else {
                            alarm.data.time.format("%p").to_string()
                        };
                        // Offer snooze only while it is actually available; the
                        // controller enforces the same predicate before snoozing.
                        let snooze_allowed = alarm.snooze_allowed();
                        if let Err(err) = compositor.broadcast_alarm_ring(
                            time,
                            period,
                            alarm.data.name,
                            snooze_allowed,
                        ) {
                            tracing::warn!(error = %err, "failed to signal alarm ring to overlay");
                        }
                    }
                    Ok(AlarmEvent::Stopped { .. } | AlarmEvent::Snoozed) => {
                        if let Err(err) = compositor.broadcast_alarm_stop() {
                            tracing::warn!(error = %err, "failed to signal alarm stop to overlay");
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "alarm event receiver lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                command = commands.recv() => match command {
                    Some(AlarmCommand::Dismiss) => alarm_bus.stop_current(),
                    Some(AlarmCommand::Snooze) => alarm_bus.snooze(),
                    None => break,
                },
            }
        }
    });
}

/// Track the ringing alarm as a `watch<bool>` for the screen auto-off inhibit:
/// `Started` → true, `Stopped`/`Snoozed` → false. Subscribes synchronously so a
/// ring fired before the task is scheduled is not missed (cf.
/// `spawn_alarm_overlay_listener`).
fn spawn_alarm_ringing_watch(alarm_bus: &AlarmBus) -> watch::Receiver<bool> {
    let mut events = alarm_bus.subscribe_events();
    let (tx, rx) = watch::channel(false);
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(AlarmEvent::Started { .. }) => {
                    let _ = tx.send(true);
                }
                Ok(AlarmEvent::Stopped { .. } | AlarmEvent::Snoozed) => {
                    let _ = tx.send(false);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "alarm ringing watch lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    rx
}

fn spawn_upgrade_display_listener(
    compositor: Arc<dyn Compositor>,
    receiver: watch::Receiver<Option<UpgradeDisplaySnapshot>>,
) {
    tokio::spawn(async move {
        forward_upgrade_display_state(receiver, |snapshot| compositor.set_upgrade_state(snapshot))
            .await;
    });
}

fn post_upgrade_kind(firmware: UpgradeMarker, service: UpgradeMarker) -> Option<UpgradeKind> {
    if firmware != UpgradeMarker::Absent {
        Some(UpgradeKind::Firmware)
    } else if service == UpgradeMarker::Consumed {
        Some(UpgradeKind::Packages)
    } else {
        None
    }
}

async fn forward_upgrade_display_state<F>(
    mut receiver: watch::Receiver<Option<UpgradeDisplaySnapshot>>,
    mut set_state: F,
) where
    F: FnMut(UpgradeDisplaySnapshot) -> Result<(), crate::compositor::CompositorError>,
{
    receiver.mark_changed();
    loop {
        if receiver.changed().await.is_err() {
            break;
        }
        if let Some(snapshot) = receiver.borrow_and_update().clone()
            && let Err(error) = set_state(snapshot)
        {
            warn!(%error, "failed to relay upgrade display state to compositor");
        }
    }
}

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
    secret_store: Arc<RwLock<SecretStoreHandle>>,
    initial_setup: InitialSetup<T, V>,
    button_manager: ButtonManager<T>,
    led_controller: LedController<T>,
    led_coordinator: LedCoordinatorHandle,
    widget_coordinator: Arc<Coordinator>,
    widget_registry: Arc<WidgetRegistry>,
    widget_reload_task: tokio::task::JoinHandle<()>,
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
        // Captured here, not where the collection job is registered: the
        // startup floor measures the boot window, and registration happens
        // hundreds of lines of service construction later.
        let started = tokio::time::Instant::now();

        let listener = TcpListener::bind(config.address).await?;

        let hardware_capabilities = compositor.hardware_capabilities();

        let state_service = StateService::new();

        let (mut config_handle, extracted_accounts) = ConfigHandle::init(
            config.config_path.clone(),
            config.default_brightness_pct,
            config.default_night_mode_brightness_pct,
            config.default_volume_pct,
            config.default_night_mode_volume_pct,
            manager.platform().product(),
        )
        .await;

        // Store first: the config may only stop carrying secrets once they are safely stored.
        // A failed store write leaves them in the on-disk config for the next boot to retry
        // — a guarantee that lasts only until the first config save writes v2 without them.
        // They also stay in the running store, so any later account edit persists them.
        let mut secret_store = SecretStoreHandle::init(&config.config_path).await;
        match secret_store.merge_extracted(extracted_accounts).await {
            Ok(false) => {}
            Ok(true) => {
                if let Err(err) = config_handle.save().await {
                    warn!(
                        ?err,
                        "failed to persist the config after account extraction"
                    );
                }
            }
            Err(err) => error!(
                ?err,
                "failed to persist extracted accounts; leaving them in the config"
            ),
        }

        let autoupgrade_config = config_handle.autoupgrade();
        let led_enabled = config_handle.led_enabled();

        let config_handle = Arc::new(RwLock::new(config_handle));
        let secret_store = Arc::new(RwLock::new(secret_store));

        let scheduler = JobScheduler::init(
            manager.watch_timezone_updates(),
            config.crontab_path.clone(),
        )
        .await;

        let (widget_manager, widget_events) =
            WidgetManager::init(config.widgets_paths.clone(), config.capture_widget_output).await;
        let widget_registry = widget_manager.registry();
        let widget_coordinator = Arc::new(Coordinator::new(
            widget_manager,
            compositor.clone(),
            widget_registry.clone(),
            hardware_capabilities,
            secret_store.clone(),
        ));
        let widget_reload_task = crate::widget::spawn_reload_signal_task(
            widget_coordinator.clone(),
            config_handle.clone(),
        )?;

        let system_upgrade_service = SystemUpgradeService::new(
            firmware_index,
            &config.upgrade_image_path,
            manager.clone(),
            state_service.clone(),
            scheduler.clone(),
            started,
            package_backend,
            Arc::new(UpgradeWidgetLifecycle::new(
                widget_coordinator.clone(),
                config_handle.clone(),
            )),
            config.pending_install_path.clone(),
        );

        spawn_upgrade_display_listener(
            compositor.clone(),
            system_upgrade_service.subscribe_display_state(),
        );
        // Consume both even though only one decides the outcome: a firmware
        // upgrade activates its generation after the reboot, so it can leave a
        // service marker that would otherwise replay later.
        let firmware_marker = manager.consume_upgrade_marker().await;
        let service_marker = manager.consume_service_upgrade_marker().await;
        if let Some(kind) = post_upgrade_kind(firmware_marker, service_marker)
            && manager
                .network_manager()
                .provisioning()
                .device_state()
                .await
                == BmcState::Operational
        {
            system_upgrade_service.publish_post_reboot_success(kind);
        }
        system_upgrade_service
            .autoupgrade_init(autoupgrade_config.enabled)
            .await;
        system_upgrade_service
            .gc_init(config.nix_gc_config_path.clone())
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

        // Subscribe the overlay listener before the controller inits: it calls
        // `subscribe_events()` synchronously, so the receiver exists before the
        // controller can emit a `Started` event a fresh subscriber would miss.
        spawn_alarm_overlay_listener(compositor.clone(), alarm_bus.clone(), config_handle.clone());
        // Same discipline for the screen auto-off inhibit: subscribe now so a
        // ring during startup can't be missed.
        let alarm_ringing = spawn_alarm_ringing_watch(&alarm_bus);

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
            alarm_bus.clone(),
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
            alarm_ringing,
        )
        .await;

        // Subscribing right after init keeps the receiver ahead of the first
        // possible auto-off, which cannot fire before the inactivity timeout.
        tokio::spawn(run_screen_blank_reset_task(
            system_manager.subscribe_screen_blanked(),
            compositor.clone(),
        ));

        tokio::spawn(run_night_mode_cycling_task(
            system_manager.subscribe_night_mode(),
            compositor.clone(),
        ));

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

        // Before the initial spawn, so an account or scene change
        // landing mid-spawn is buffered rather than never observed.
        {
            let scenes_rx = config_handle.read().await.subscribe_scenes_change();
            let accounts_rx = secret_store.read().await.subscribe_accounts_change();
            crate::widget::coordinator::start_credential_listener(
                widget_coordinator.clone(),
                config_handle.clone(),
                scenes_rx,
                accounts_rx,
            );
        }

        crate::widget::coordinator::start_widget_event_listener(
            widget_coordinator.clone(),
            widget_events,
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
        crate::widget::coordinator::start_volume_listener(
            compositor.clone(),
            config_handle.clone(),
            config_handle.read().await.subscribe_sound_settings_change(),
            system_manager.subscribe_night_mode(),
        );
        crate::widget::coordinator::start_night_mode_listener(
            compositor.clone(),
            system_manager.clone(),
        );
        crate::widget::coordinator::start_wifi_reconfig_listener(
            compositor.clone(),
            manager.clone(),
            manager
                .network_manager()
                .provisioning()
                .watch_setup_ap_active(),
        );

        Ok(Self {
            listener,
            manager,
            session_manager: session_manager.into(),
            config,
            system_upgrade_service,
            config_handle,
            secret_store,
            initial_setup,
            button_manager,
            led_controller,
            led_coordinator,
            widget_coordinator,
            widget_registry,
            widget_reload_task,
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

        let widget_reload_task = self.widget_reload_task;
        let server_result = WebService::new(
            self.manager,
            self.session_manager,
            self.config.server_config,
            self.system_upgrade_service,
            self.config_handle,
            self.secret_store,
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
        .await;

        // In case the app panics, this is not executed.
        // The children are SIGKILL'd thanks to kill_on_drop(true).
        widget_reload_task.abort();
        self.widget_coordinator.stop_all().await;
        server_result?;
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
    pub nix_gc_config_path: PathBuf,
    pub nix_profile_dir: PathBuf,
    pub pending_install_path: PathBuf,
    pub nix_hooks_dir: String,
    pub nix_hooks_override_path: Option<PathBuf>,
}

impl Configuration {
    const UPGRADE_IMAGE_PATH: &str = "/tmp/firmware.tar";
    const CONFIG_PATH: &str = "/etc/bmc/config.json";
    const DEFAULT_BRIGHTNESS_PCT: u8 = 60;
    const DEFAULT_NIGHT_MODE_BRIGHTNESS_PCT: u8 = 25;
    const DEFAULT_VOLUME_PCT: u8 = 60;
    const DEFAULT_NIGHT_MODE_VOLUME_PCT: u8 = 40;
    const SOUNDS_DIR: &str = match option_env!("BMC_SOUNDS_DIR") {
        Some(p) => p,
        None => "/usr/share/bmc/sounds/",
    };
    const CRONTAB_PATH: &str = "/etc/crontabs/root";
    const NIX_SERVERS_CONFIG_PATH: &str = "/etc/nix-upgrade/servers.json";
    const NIX_GC_CONFIG_PATH: &str = "/etc/nix-upgrade/gc.json";
    const NIX_PROFILE_DIR: &str = "/nix/var/nix/gcroots/profiles/bmc";
    // Handoff for a firmware-carried widget install: bmc writes it here, the
    // sysupgrade sequence consumes it via `bmc-nix-cli upgrade --install-from`
    // before the flash in the same boot, so it never crosses the reboot and its
    // tmpfs location is fine. See docs/devel/firmware-package-interlinking.md.
    const PENDING_INSTALL_PATH: &str = "/dev/shm/bmc-nix-pending-install.json";
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
            nix_gc_config_path: PathBuf::from(Self::NIX_GC_CONFIG_PATH),
            nix_profile_dir: PathBuf::from(Self::NIX_PROFILE_DIR),
            pending_install_path: PathBuf::from(Self::PENDING_INSTALL_PATH),
            nix_hooks_dir: Self::NIX_HOOKS_DIR.to_owned(),
            nix_hooks_override_path: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{forward_upgrade_display_state, post_upgrade_kind};
    use crate::compositor::{
        CompositorError, UpgradeDisplaySnapshot, UpgradeDisplayState, UpgradeGeneration,
        UpgradeKind,
    };
    use crate::manager::UpgradeMarker;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::sync::{Notify, watch};

    fn snapshot(generation: usize) -> UpgradeDisplaySnapshot {
        UpgradeDisplaySnapshot {
            generation: UpgradeGeneration::new(generation),
            state: UpgradeDisplayState::Succeeded {
                kind: UpgradeKind::Firmware,
            },
        }
    }

    #[tokio::test]
    async fn upgrade_bridge_replays_a_terminal_snapshot_present_before_its_first_poll() {
        let (sender, receiver) = watch::channel(None);
        let second = snapshot(2);
        sender
            .send(Some(second.clone()))
            .expect("BUG: receiver is live");
        drop(sender);
        let received = Arc::new(Mutex::new(Vec::new()));

        forward_upgrade_display_state(receiver, {
            let received = Arc::clone(&received);
            move |state| {
                received
                    .lock()
                    .expect("BUG: received lock poisoned")
                    .push(state);
                Ok(())
            }
        })
        .await;

        assert_eq!(
            *received.lock().expect("BUG: received lock poisoned"),
            vec![second]
        );
    }

    #[tokio::test]
    async fn upgrade_bridge_coalesces_to_the_latest_authoritative_snapshot() {
        let (sender, receiver) = watch::channel(None);
        sender
            .send(Some(snapshot(1)))
            .expect("BUG: receiver is live");
        let second = snapshot(2);
        sender
            .send(Some(second.clone()))
            .expect("BUG: receiver is live");
        drop(sender);
        let received = Arc::new(Mutex::new(Vec::new()));

        forward_upgrade_display_state(receiver, {
            let received = Arc::clone(&received);
            move |state| {
                received
                    .lock()
                    .expect("BUG: received lock poisoned")
                    .push(state);
                Ok(())
            }
        })
        .await;

        assert_eq!(
            *received.lock().expect("BUG: received lock poisoned"),
            vec![second]
        );
    }

    #[tokio::test]
    async fn upgrade_bridge_continues_after_a_compositor_error() {
        let (sender, receiver) = watch::channel(None);
        let first = snapshot(1);
        let second = snapshot(2);
        let entered = Arc::new(Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let received = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn(forward_upgrade_display_state(receiver, {
            let entered = Arc::clone(&entered);
            let calls = Arc::clone(&calls);
            let received = Arc::clone(&received);
            move |state| {
                received
                    .lock()
                    .expect("BUG: received lock poisoned")
                    .push(state);
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    entered.notify_one();
                    Err(CompositorError::NotStarted)
                } else {
                    Ok(())
                }
            }
        }));

        sender.send(Some(first)).expect("BUG: receiver is live");
        entered.notified().await;
        sender
            .send(Some(second.clone()))
            .expect("BUG: receiver is live");
        drop(sender);
        task.await.expect("BUG: bridge task must finish");

        assert_eq!(
            *received.lock().expect("BUG: received lock poisoned"),
            vec![snapshot(1), second]
        );
    }

    #[test]
    fn service_marker_alone_reports_a_package_upgrade() {
        assert_eq!(
            post_upgrade_kind(UpgradeMarker::Absent, UpgradeMarker::Consumed),
            Some(UpgradeKind::Packages),
            "a restart without a firmware marker can only come from a package upgrade"
        );
    }

    #[test]
    fn firmware_marker_wins_even_when_removal_fails() {
        assert_eq!(
            post_upgrade_kind(UpgradeMarker::RemovalFailed, UpgradeMarker::Consumed),
            Some(UpgradeKind::Firmware),
            "the marker's existence proves the firmware upgrade; \
             a failed removal must not demote it to packages"
        );
    }

    #[test]
    fn absent_firmware_and_unconsumed_service_marker_report_no_upgrade() {
        for service in [UpgradeMarker::Absent, UpgradeMarker::RemovalFailed] {
            assert_eq!(post_upgrade_kind(UpgradeMarker::Absent, service), None);
        }
    }
}
