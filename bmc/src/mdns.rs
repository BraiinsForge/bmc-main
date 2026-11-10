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

//! Advertises the web UI over mDNS, so the device answers at
//! `<hostname>.local` and shows up when browsing `_http._tcp`.

use std::sync::Arc;
use std::time::Duration;

use bmc_net::NetworkConfig;
use bmc_net_mdns::TxtValues;
use bmc_net_mdns::service::{MdnsService, ServiceIdentity};
use bmc_platform::BosVersion;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::manager::BmcManager;
use crate::shutdown::MDNS_GOODBYE_WAIT;

pub(crate) struct Advertiser {
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    /// The service treats a dropped sender as a shutdown.
    _enabled: watch::Sender<bool>,
}

impl Advertiser {
    pub(crate) async fn start<T: BmcManager>(manager: &T, port: u16, device: &str) -> Self {
        let txt_values = deck_identity(
            device,
            manager.version().await,
            manager.packages_version().await,
        );
        let network: Arc<dyn NetworkConfig> = manager.network_manager_arc();
        let shutdown = CancellationToken::new();
        let (enabled_tx, enabled) = watch::channel(true);
        let task = MdnsService::new(
            network,
            ServiceIdentity { port, txt_values },
            Duration::ZERO,
        )
        .spawn(enabled, shutdown.clone());
        Self {
            shutdown,
            task,
            _enabled: enabled_tx,
        }
    }

    /// Cancelling it withdraws the advertisement.
    pub(crate) fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    /// Waits for the goodbye, so the process does not exit under it and
    /// leave peers listing a device that is gone.
    pub(crate) async fn stop(self) {
        self.shutdown.cancel();
        match tokio::time::timeout(MDNS_GOODBYE_WAIT, self.task).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => warn!(%error, "mDNS task failed before shutdown"),
            Err(_) => warn!("mDNS goodbye still pending after {MDNS_GOODBYE_WAIT:?}, exiting"),
        }
    }
}

fn deck_identity(
    device: &str,
    firmware: Option<BosVersion>,
    packages: Option<String>,
) -> TxtValues {
    TxtValues::Deck {
        device: device.to_owned(),
        fw_version: firmware.map(|version| version.full),
        bmc_version: packages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The web UI shows the full firmware string, and the advertised
    /// version has to match what the owner reads there.
    #[test]
    fn deck_identity_advertises_the_full_firmware_version() {
        let identity = deck_identity(
            "Braiins Deck",
            Some(BosVersion::new(&"2026-10-21-0-9ed62a93-26.09", &"26.09")),
            Some("2026.11.05-4ec47a20".to_owned()),
        );
        let TxtValues::Deck {
            device,
            fw_version,
            bmc_version,
        } = identity
        else {
            panic!("BUG: a Deck must not advertise as BOS: {identity:?}");
        };
        assert_eq!(device, "Braiins Deck");
        assert_eq!(fw_version.as_deref(), Some("2026-10-21-0-9ed62a93-26.09"));
        assert_eq!(bmc_version.as_deref(), Some("2026.11.05-4ec47a20"));
    }
}
