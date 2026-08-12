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

//! Device provisioning state: the factory-default / setup-pending /
//! wifi-reconfiguration flags and the [`BmcState`] derived from them.
//!
//! This is a device-lifecycle concern, not a networking one, so it lives
//! behind its own [`ProvisioningState`] trait. A [`NetworkManager`] holds one
//! and reads it to decide setup-AP / captive-portal behaviour.
//!
//! [`NetworkManager`]: crate::NetworkManager

use async_trait::async_trait;
use bmc_net_types::network::BmcState;
use tokio::sync::watch;

use crate::command::{BOS_DEFAULTS_LIB, run_sourced, run_sourced_succeeds};

/// The device provisioning state machine: the persistent flags and the
/// [`BmcState`] they resolve to, plus the wifi-reconfiguration transitions.
#[async_trait]
pub trait ProvisioningState: Send + Sync + std::fmt::Debug {
    /// Whether the device has never been set up.
    async fn is_factory_default(&self) -> bool;
    /// Whether first-time setup is still pending.
    async fn is_setup_pending(&self) -> bool;
    /// Whether the device is temporarily re-running WiFi setup.
    async fn is_wifi_reconfig(&self) -> bool;

    /// Resolves the current [`BmcState`].
    ///
    /// NOTE: factory-default deliberately outranks a lingering wifi-reconfig
    /// flag, so an unconfigured device is never merely "reconfiguring". The two
    /// flags are normally mutually exclusive, and when they do coexist the
    /// safer report is "factory default". [`Self::advance`]'s `FactoryDefault`
    /// arm clears both flags to stay consistent with this ordering.
    async fn device_state(&self) -> BmcState {
        if self.is_factory_default().await {
            BmcState::FactoryDefault
        } else if self.is_wifi_reconfig().await {
            BmcState::WifiReconfiguration
        } else if self.is_setup_pending().await {
            BmcState::SetupPending
        } else {
            BmcState::Operational
        }
    }

    /// Clears the flag for the current transient state, advancing the device
    /// towards `Operational` (called once setup / reconfiguration completes).
    async fn advance(&self) -> anyhow::Result<()>;
    /// Sets the wifi-reconfiguration flag.
    ///
    /// This deliberately does not publish to watchers: the setup AP is not up
    /// yet at this point. Callers announce it with
    /// [`publish_setup_ap_active`] once activation has actually succeeded.
    ///
    /// [`publish_setup_ap_active`]: ProvisioningState::publish_setup_ap_active
    async fn mark_wifi_reconfig(&self) -> anyhow::Result<()>;
    /// Publishes whether the setup AP is currently up to watchers.
    fn publish_setup_ap_active(&self, active: bool);
    /// Clears the wifi-reconfiguration flag and publishes it to watchers.
    async fn clear_wifi_reconfig(&self) -> anyhow::Result<()>;

    /// Watches whether the setup AP is active (i.e. the device is in
    /// `FactoryDefault` or `WifiReconfiguration`); the current value is seeded
    /// on subscribe, so late subscribers observe the real state.
    fn watch_setup_ap_active(&self) -> watch::Receiver<bool>;
}

/// [`ProvisioningState`] backed by the on-device `bos-defaults.sh` flag helpers.
#[derive(Debug)]
pub struct UciProvisioningState {
    setup_ap_active_sender: watch::Sender<bool>,
}

impl UciProvisioningState {
    /// Builds the state and seeds the "setup AP active" watch from real state,
    /// so the first subscriber sees the true value. Both `FactoryDefault` and
    /// `WifiReconfiguration` run the setup AP.
    pub async fn new() -> Self {
        let (setup_ap_active_sender, _) = watch::channel(false);
        let state = Self {
            setup_ap_active_sender,
        };
        let setup_ap_active = matches!(
            state.device_state().await,
            BmcState::FactoryDefault | BmcState::WifiReconfiguration
        );
        // `send_replace` (not `send`) so the stored value updates even though no
        // receiver has subscribed yet.
        state.setup_ap_active_sender.send_replace(setup_ap_active);
        state
    }
}

#[async_trait]
impl ProvisioningState for UciProvisioningState {
    async fn is_factory_default(&self) -> bool {
        defaults_predicate("is_factory_default").await
    }

    async fn is_setup_pending(&self) -> bool {
        defaults_predicate("is_setup_pending").await
    }

    async fn is_wifi_reconfig(&self) -> bool {
        defaults_predicate("is_wifi_reconfig").await
    }

    async fn advance(&self) -> anyhow::Result<()> {
        match self.device_state().await {
            BmcState::FactoryDefault => {
                // Announce the setup AP is gone before the fallible unset calls:
                // if the second unset fails, the watch must not keep advertising
                // a setup AP we have already started tearing down.
                self.setup_ap_active_sender.send_replace(false);
                run_defaults_script("unset_factory_default").await?;
                // Clear any stale wifi-reconfig flag too, consistent with
                // factory-default outranking it in `device_state`.
                run_defaults_script("unset_wifi_reconfig").await?;
            }
            BmcState::SetupPending => run_defaults_script("unset_setup_pending").await?,
            BmcState::WifiReconfiguration => {
                self.setup_ap_active_sender.send_replace(false);
                run_defaults_script("unset_wifi_reconfig").await?;
            }
            BmcState::Operational => {}
        }
        Ok(())
    }

    async fn mark_wifi_reconfig(&self) -> anyhow::Result<()> {
        // Only the flag is written here. Publishing the watch before the AP is
        // configured makes the settings tray resolve `ap_ssid` against a radio
        // that is still in station mode; it reads once per transition, so the
        // `None` it gets back sticks until the next one. `send_replace(true)`
        // therefore belongs after activation succeeds, not here.
        run_defaults_script("set_wifi_reconfig").await
    }

    fn publish_setup_ap_active(&self, active: bool) {
        self.setup_ap_active_sender.send_replace(active);
    }

    async fn clear_wifi_reconfig(&self) -> anyhow::Result<()> {
        self.setup_ap_active_sender.send_replace(false);
        run_defaults_script("unset_wifi_reconfig").await?;
        Ok(())
    }

    fn watch_setup_ap_active(&self) -> watch::Receiver<bool> {
        self.setup_ap_active_sender.subscribe()
    }
}

/// Inert [`ProvisioningState`] for boards without a provisioning flow (e.g.
/// buildroot miners): always `Operational`, every transition a no-op.
#[derive(Debug)]
pub struct NoProvisioning {
    setup_ap_active_sender: watch::Sender<bool>,
}

impl Default for NoProvisioning {
    fn default() -> Self {
        Self {
            setup_ap_active_sender: watch::channel(false).0,
        }
    }
}

#[async_trait]
impl ProvisioningState for NoProvisioning {
    async fn is_factory_default(&self) -> bool {
        false
    }

    async fn is_setup_pending(&self) -> bool {
        false
    }

    async fn is_wifi_reconfig(&self) -> bool {
        false
    }

    async fn advance(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn mark_wifi_reconfig(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn publish_setup_ap_active(&self, _active: bool) {}

    async fn clear_wifi_reconfig(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn watch_setup_ap_active(&self) -> watch::Receiver<bool> {
        self.setup_ap_active_sender.subscribe()
    }
}

/// Stateful in-memory [`ProvisioningState`] for tests and the mock backend: the
/// three flags live in a watch channel so `device_state` transitions are
/// observable.
#[derive(Debug)]
pub struct MockProvisioningState {
    factory_default: watch::Sender<bool>,
    setup_pending: watch::Sender<bool>,
    wifi_reconfig: watch::Sender<bool>,
    /// Derived `factory_default || wifi_reconfig`, kept in sync on every flag
    /// change so [`Self::watch_setup_ap_active`] mirrors the Uci backend.
    setup_ap_active: watch::Sender<bool>,
}

impl Default for MockProvisioningState {
    fn default() -> Self {
        Self {
            factory_default: watch::channel(false).0,
            setup_pending: watch::channel(false).0,
            wifi_reconfig: watch::channel(false).0,
            setup_ap_active: watch::channel(false).0,
        }
    }
}

impl MockProvisioningState {
    /// Seeds the initial flags (e.g. a fresh device starts factory-default).
    #[must_use]
    pub fn new(factory_default: bool, setup_pending: bool) -> Self {
        let state = Self::default();
        state.factory_default.send_replace(factory_default);
        state.setup_pending.send_replace(setup_pending);
        state.refresh_setup_ap_active();
        state
    }

    /// Recomputes the derived "setup AP active" value from the raw flags.
    fn refresh_setup_ap_active(&self) {
        let active = *self.factory_default.borrow() || *self.wifi_reconfig.borrow();
        self.setup_ap_active.send_replace(active);
    }
}

#[async_trait]
impl ProvisioningState for MockProvisioningState {
    async fn is_factory_default(&self) -> bool {
        *self.factory_default.borrow()
    }

    async fn is_setup_pending(&self) -> bool {
        *self.setup_pending.borrow()
    }

    async fn is_wifi_reconfig(&self) -> bool {
        *self.wifi_reconfig.borrow()
    }

    async fn advance(&self) -> anyhow::Result<()> {
        match self.device_state().await {
            BmcState::FactoryDefault => {
                self.factory_default.send_replace(false);
                self.wifi_reconfig.send_replace(false);
            }
            BmcState::SetupPending => {
                self.setup_pending.send_replace(false);
            }
            BmcState::WifiReconfiguration => {
                self.wifi_reconfig.send_replace(false);
            }
            BmcState::Operational => {}
        }
        self.refresh_setup_ap_active();
        Ok(())
    }

    async fn mark_wifi_reconfig(&self) -> anyhow::Result<()> {
        // Mirrors the Uci backend: the flag moves now, the setup-AP watch only
        // once activation has succeeded.
        self.wifi_reconfig.send_replace(true);
        Ok(())
    }

    fn publish_setup_ap_active(&self, active: bool) {
        self.setup_ap_active.send_replace(active);
    }

    async fn clear_wifi_reconfig(&self) -> anyhow::Result<()> {
        self.wifi_reconfig.send_replace(false);
        self.refresh_setup_ap_active();
        Ok(())
    }

    fn watch_setup_ap_active(&self) -> watch::Receiver<bool> {
        self.setup_ap_active.subscribe()
    }
}

async fn run_defaults_script(snippet: &str) -> anyhow::Result<()> {
    run_sourced(BOS_DEFAULTS_LIB, snippet).await
}

/// Evaluate a bos-defaults predicate (`is_*`), returning its boolean result.
///
/// A non-zero exit means "no". If the script cannot be evaluated at all (the
/// interpreter or defaults library is missing), that is logged and treated as
/// `false` rather than being silently indistinguishable from a genuine "no".
async fn defaults_predicate(snippet: &str) -> bool {
    match run_sourced_succeeds(BOS_DEFAULTS_LIB, snippet).await {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!("could not evaluate defaults predicate `{snippet}`: {e:#}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BmcState, MockProvisioningState, ProvisioningState};

    /// Regression: the setup-AP watch must not fire before the AP is up.
    ///
    /// Watchers resolve the AP SSID once per transition, so a `true` published
    /// while the radio is still in station mode leaves the settings tray stuck
    /// on its previous value until the next transition, which never comes.
    #[tokio::test]
    async fn setup_ap_watch_stays_quiet_until_activation_is_announced() {
        let state = MockProvisioningState::new(false, false);
        let mut watcher = state.watch_setup_ap_active();
        assert!(!*watcher.borrow_and_update());

        state
            .mark_wifi_reconfig()
            .await
            .expect("BUG: marking wifi reconfig must succeed");

        assert_eq!(state.device_state().await, BmcState::WifiReconfiguration);
        assert!(
            !watcher.has_changed().expect("BUG: sender must be alive"),
            "setup AP announced before it was brought up"
        );

        state.publish_setup_ap_active(true);

        assert!(watcher.has_changed().expect("BUG: sender must be alive"));
        assert!(*watcher.borrow_and_update());
    }

    /// Leaving reconfiguration must still take the announcement back down.
    #[tokio::test]
    async fn clearing_wifi_reconfig_retracts_the_setup_ap_announcement() {
        let state = MockProvisioningState::new(false, false);
        state
            .mark_wifi_reconfig()
            .await
            .expect("BUG: marking wifi reconfig must succeed");
        state.publish_setup_ap_active(true);

        let mut watcher = state.watch_setup_ap_active();
        assert!(*watcher.borrow_and_update());

        state
            .clear_wifi_reconfig()
            .await
            .expect("BUG: clearing wifi reconfig must succeed");

        assert!(!*watcher.borrow_and_update());
        assert_eq!(state.device_state().await, BmcState::Operational);
    }
}
