// Copyright (C) 2026  Braiins Systems s.r.o.
//
// This file is part of Braiins Open-Source Initiative (BOSI).
//
// BOSI is free software: you can redistribute it and/or modify
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
// Please, keep in mind that we may also license BOSI or any part thereof
// under a proprietary license. For more information on the terms and conditions
// of such proprietary license or if you have any other questions, please
// contact us at opensource@braiins.com.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Mutex;

use async_trait::async_trait;
use bmc_net_types::network::{
    IfaceData, InitialSetupError, NetworkInfo, NetworkProtocolConfig, WifiData, WifiEvent,
    WifiNetworkConfig,
};
use bmc_net_types::wifi::{EncryptionType, WifiScanItem, WifiStatus};
use tokio::sync::broadcast;

use crate::provisioning::{MockProvisioningState, ProvisioningState};
use crate::{NetworkConfig, NetworkManager, WifiControl};

/// Stateful in-memory [`NetworkManager`] for mock platforms and tests: setters
/// update observable state, so callers can exercise the config and
/// provisioning state-machine contracts rather than reading back constants.
#[derive(Debug)]
pub struct MockNetworkManager {
    hostname: Mutex<Option<String>>,
    network_config: Mutex<NetworkProtocolConfig>,
    connected_wifi: Mutex<Option<WifiNetworkConfig>>,
    wifi_enabled: Mutex<bool>,
    wifi_event_sender: broadcast::Sender<WifiEvent>,
    provisioning: MockProvisioningState,
}

impl Default for MockNetworkManager {
    fn default() -> Self {
        Self {
            hostname: Mutex::new(Some("mock".to_owned())),
            network_config: Mutex::new(NetworkProtocolConfig::Dhcp),
            connected_wifi: Mutex::new(None),
            wifi_enabled: Mutex::new(true),
            wifi_event_sender: broadcast::channel(1).0,
            provisioning: MockProvisioningState::default(),
        }
    }
}

impl MockNetworkManager {
    /// Builds a mock seeded with a provisioning state, so callers (e.g.
    /// bmc-mock) can start it factory-default or setup-pending.
    #[must_use]
    pub fn with_provisioning(factory_default: bool, setup_pending: bool) -> Self {
        Self {
            provisioning: MockProvisioningState::new(factory_default, setup_pending),
            ..Self::default()
        }
    }
}

fn lock<T>(guard: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    guard.lock().expect("BUG: mock network state lock poisoned")
}

#[async_trait]
impl NetworkConfig for MockNetworkManager {
    async fn hostname(&self) -> Option<String> {
        lock(&self.hostname).clone()
    }

    fn mac_address(&self) -> Option<String> {
        Some("02:00:00:00:00:01".to_owned())
    }

    async fn ip_address(&self) -> Option<IpAddr> {
        Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
    }

    async fn network_config(&self) -> Option<NetworkProtocolConfig> {
        Some(lock(&self.network_config).clone())
    }

    async fn set_network_config(&self, config: NetworkProtocolConfig) -> anyhow::Result<()> {
        *lock(&self.network_config) = config;
        Ok(())
    }

    async fn set_hostname(&self, hostname: String) -> anyhow::Result<()> {
        crate::validate_hostname(&hostname)?;
        *lock(&self.hostname) = Some(hostname);
        Ok(())
    }

    async fn network_info(&self) -> anyhow::Result<NetworkInfo> {
        Ok(NetworkInfo {
            hostname: self.hostname().await,
            mac_address: self.mac_address().and_then(|mac| mac.parse().ok()),
            ..NetworkInfo::default()
        })
    }

    fn eth_data(&self) -> IfaceData {
        IfaceData::default()
    }
}

#[async_trait]
impl WifiControl for MockNetworkManager {
    async fn scan(&self) -> anyhow::Result<Vec<WifiScanItem>> {
        Ok(vec![WifiScanItem::new(
            "MockAP".to_owned(),
            -50,
            EncryptionType::Wpa2,
        )])
    }

    async fn status(&self) -> anyhow::Result<WifiData> {
        // Reflect the stored radio flag so `set_wifi_enabled` is observable.
        Ok(WifiData {
            status: WifiStatus {
                enabled: *lock(&self.wifi_enabled),
                ..WifiStatus::default()
            },
            ..WifiData::default()
        })
    }

    async fn saved_networks(&self) -> anyhow::Result<Vec<WifiStatus>> {
        Ok(Vec::new())
    }

    async fn ssid(&self) -> anyhow::Result<String> {
        Ok(lock(&self.connected_wifi)
            .as_ref()
            .map_or_else(|| "MockAP".to_owned(), |config| config.ssid.clone()))
    }

    async fn wifi_save_and_connect(
        &self,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> anyhow::Result<()> {
        *lock(&self.connected_wifi) = Some(WifiNetworkConfig {
            ssid,
            password,
            encryption,
        });
        Ok(())
    }

    async fn set_wifi_enabled(&self, enable: bool) -> anyhow::Result<()> {
        *lock(&self.wifi_enabled) = enable;
        Ok(())
    }

    async fn init_wifi_ap(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop_wifi_ap(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn wifi_initial_setup(&self, config: WifiNetworkConfig) -> Result<(), InitialSetupError> {
        *lock(&self.connected_wifi) = Some(config);
        self.provisioning
            .advance()
            .await
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))
    }

    async fn revert_to_initial_setup(&self) -> Result<(), InitialSetupError> {
        Ok(())
    }

    async fn enter_wifi_reconfiguration(&self) -> Result<(), InitialSetupError> {
        self.provisioning
            .mark_wifi_reconfig()
            .await
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))
    }

    async fn exit_wifi_reconfiguration(&self) -> Result<(), InitialSetupError> {
        self.provisioning
            .clear_wifi_reconfig()
            .await
            .map_err(|e| InitialSetupError::UnexpectedFailure(e.to_string()))
    }

    async fn captive_portal_redirect_host(&self) -> Option<String> {
        None
    }

    fn subscribe_wifi_events(&self) -> broadcast::Receiver<WifiEvent> {
        self.wifi_event_sender.subscribe()
    }
}

impl NetworkManager for MockNetworkManager {
    fn wifi(&self) -> Option<&dyn WifiControl> {
        Some(self)
    }

    fn provisioning(&self) -> &dyn ProvisioningState {
        &self.provisioning
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_net_types::network::BmcState;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("BUG: failed to build test runtime")
            .block_on(future)
    }

    #[test]
    fn setters_and_provisioning_transitions_are_observable() {
        block_on(async {
            let mock = MockNetworkManager {
                provisioning: MockProvisioningState::new(true, false),
                ..MockNetworkManager::default()
            };

            // Hostname setter is observable.
            mock.set_hostname("deck".to_owned())
                .await
                .expect("set hostname");
            assert_eq!(mock.hostname().await.as_deref(), Some("deck"));

            // The provisioning state machine advances observably, and mirrors
            // the device's FactoryDefault -> SetupPending -> Operational path.
            assert_eq!(
                mock.provisioning().device_state().await,
                BmcState::FactoryDefault
            );
            mock.provisioning().advance().await.expect("advance");
            assert_eq!(
                mock.provisioning().device_state().await,
                BmcState::SetupPending
            );
            mock.provisioning().advance().await.expect("advance");
            assert_eq!(
                mock.provisioning().device_state().await,
                BmcState::Operational
            );
        });
    }
}
