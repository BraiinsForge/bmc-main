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

mod captive_portal;
mod grpc;
mod http_server;
mod no_password;
mod session;

use crate::alarm::AlarmController;
use crate::backlight::DisplayBacklightDriver;
use crate::config::ConfigHandle;
use crate::initial_setup::InitialSetup;
use crate::led::LedController;
use crate::led_coordinator::LedCoordinatorHandle;
use crate::secret_store::SecretStoreHandle;
use crate::session::Manager as SessionManager;
use crate::shutdown::{DRAIN_DEADLINE, DRAIN_QUIET};
use crate::sound::SoundController;
use crate::system_manager::SystemManager;
use crate::widget::{Coordinator, WidgetRegistry};
use crate::{BmcManager, system_upgrade::SystemUpgradeService};
use anyhow::Result;
use axum::{ServiceExt, extract::Request, http::header::CONTENT_TYPE};
use bmc_platform::HardwareCapabilities;
use bmc_upgrade::firmware::FirmwareIndex;
use std::{io, net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::net::TcpListener;
use tokio::sync::{RwLock, oneshot};
use tower::{Layer, steer::Steer};
use tower_http::normalize_path::NormalizePathLayer;
use tracing::warn;

pub(crate) struct WebService<
    T: BmcManager,
    S: SessionManager,
    U: FirmwareIndex,
    V: DisplayBacklightDriver,
> {
    manager: Arc<T>,
    session_manager: Arc<S>,
    config: ServerConfig,
    system_upgrade_service: SystemUpgradeService<U, T>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    secret_store: Arc<RwLock<SecretStoreHandle>>,
    initial_setup: InitialSetup<T, U>,
    led_controller: LedController<T>,
    widget_registry: Arc<WidgetRegistry>,
    widget_coordinator: Arc<Coordinator>,
    led_coordinator: LedCoordinatorHandle,
    system_manager: SystemManager<V>,
    sound_controller: SoundController,
    alarm_controller: AlarmController,
    hardware_capabilities: HardwareCapabilities,
}

impl<T: BmcManager, S: SessionManager, U: FirmwareIndex, V: DisplayBacklightDriver>
    WebService<T, S, U, V>
{
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn new(
        manager: Arc<T>,
        session_manager: Arc<S>,
        config: ServerConfig,
        system_upgrade_service: SystemUpgradeService<U, T>,
        config_handle: Arc<RwLock<ConfigHandle>>,
        secret_store: Arc<RwLock<SecretStoreHandle>>,
        initial_setup: InitialSetup<T, U>,
        led_controller: LedController<T>,
        widget_registry: Arc<WidgetRegistry>,
        widget_coordinator: Arc<Coordinator>,
        led_coordinator: LedCoordinatorHandle,
        system_manager: SystemManager<V>,
        sound_controller: SoundController,
        alarm_controller: AlarmController,
        hardware_capabilities: HardwareCapabilities,
    ) -> Self {
        Self {
            manager,
            session_manager,
            config,
            system_upgrade_service,
            config_handle,
            secret_store,
            initial_setup,
            led_controller,
            widget_registry,
            widget_coordinator,
            led_coordinator,
            system_manager,
            sound_controller,
            alarm_controller,
            hardware_capabilities,
        }
    }

    pub(crate) async fn run(self, listener: TcpListener) -> Result<()> {
        let http_router = http_server::HttpServer::new(
            self.config,
            self.manager.clone(),
            self.widget_registry.clone(),
        )
        .build();
        let grpc_router = grpc::GrpcWeb::new(
            self.manager.clone(),
            self.session_manager.clone(),
            self.system_upgrade_service,
            self.config_handle,
            self.secret_store,
            self.initial_setup,
            self.led_controller,
            self.widget_registry,
            self.widget_coordinator,
            self.led_coordinator,
            self.system_manager,
            self.sound_controller,
            self.alarm_controller,
            self.hardware_capabilities,
        )
        .build()
        .into_axum_router()
        .layer(session::SessionLayer::new(self.session_manager.clone()))
        .layer(no_password::NoPasswordLayer::new(self.session_manager))
        .layer(tower_cookies::CookieManagerLayer::new());

        // combine grpc and http router into one service
        let service = Steer::new(
            vec![http_router, grpc_router],
            |req: &Request, _services: &[_]| {
                // grpc service -> 1
                // http service -> 0
                usize::from(
                    req.headers()
                        .get(CONTENT_TYPE)
                        .map(axum::http::HeaderValue::as_bytes)
                        .as_ref()
                        .is_some_and(|content_type| content_type.starts_with(b"application/grpc")),
                )
            },
        );

        let service = NormalizePathLayer::trim_trailing_slash().layer(service);
        let service =
            ServiceExt::<Request>::into_make_service_with_connect_info::<SocketAddr>(service);

        let (signalled_tx, signalled) = oneshot::channel();
        let server = axum::serve(listener, service)
            .with_graceful_shutdown(async move {
                self.manager.handle_graceful_shutdown().await;
                let _ = signalled_tx.send(());
            })
            .into_future();

        serve_until_drained(server, signalled).await?;

        Ok(())
    }
}

/// Serves until the shutdown signal, then drains on a leash.
///
/// An idle HTTP keep-alive counts as open until the peer hangs up,
/// so one parked browser tab keeps a graceful drain pending forever.
/// An unbounded drain only trades a clean exit for a killed one,
/// with nothing in the log to say why; [`crate::shutdown`] holds the budget.
async fn serve_until_drained(
    server: impl Future<Output = io::Result<()>>,
    shutdown_signalled: oneshot::Receiver<()>,
) -> io::Result<()> {
    tokio::pin!(server);

    tokio::select! {
        result = &mut server => return result,
        _ = shutdown_signalled => {}
    }

    tokio::select! {
        result = &mut server => return result,
        () = tokio::time::sleep(DRAIN_QUIET) => {}
    }
    warn!(
        "shutdown is waiting for open connections to close; an idle browser tab is the usual cause"
    );

    tokio::select! {
        result = &mut server => result,
        () = tokio::time::sleep(DRAIN_DEADLINE.saturating_sub(DRAIN_QUIET)) => {
            warn!("connections still open after {DRAIN_DEADLINE:?}, exiting without them");
            Ok(())
        }
    }
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub www_root_path: PathBuf,
    pub www_assets_path: PathBuf,
    pub www_var_path: PathBuf,
}

impl ServerConfig {
    #[must_use]
    pub fn set_www_root_path(mut self, www_root_path: PathBuf) -> Self {
        self.www_root_path = www_root_path;
        self
    }

    #[must_use]
    pub fn set_www_assets_path(mut self, www_assets_path: PathBuf) -> Self {
        self.www_assets_path = www_assets_path;
        self
    }

    #[must_use]
    pub fn set_www_var_path(mut self, www_var_path: PathBuf) -> Self {
        self.www_var_path = www_var_path;
        self
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        const DEFAULT_ROOT: &str = match option_env!("BMC_WEB_FRONTEND_DIR") {
            Some(p) => p,
            None => "/run/current-profile/www/bmc",
        };
        Self {
            www_root_path: PathBuf::from(DEFAULT_ROOT),
            www_assets_path: PathBuf::from(DEFAULT_ROOT).join("assets"),
            www_var_path: PathBuf::from(DEFAULT_ROOT).join("var"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{future::pending, time::Duration};
    use tokio::time::Instant;

    #[tokio::test(start_paused = true)]
    async fn a_drain_that_never_finishes_is_cut_at_the_deadline() {
        let (tx, rx) = oneshot::channel();
        tx.send(()).expect("BUG: the receiver is alive below");

        let started = Instant::now();
        serve_until_drained(pending(), rx)
            .await
            .expect("BUG: giving up on a drain is not an error");

        assert_eq!(
            started.elapsed(),
            DRAIN_DEADLINE,
            "a connection held open must not delay exit past the deadline"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_server_error_is_reported_without_waiting_for_a_signal() {
        // `_tx` stays bound: dropping it resolves the receiver and starts a drain.
        let (_tx, rx) = oneshot::channel();
        let failed = async { Err(io::Error::other("listener died")) };

        let started = Instant::now();
        let error = serve_until_drained(failed, rx)
            .await
            .expect_err("BUG: the server future yielded an error");

        assert_eq!(error.to_string(), "listener died");
        assert_eq!(
            started.elapsed(),
            Duration::ZERO,
            "no drain is owed before a shutdown signal"
        );
    }
}
