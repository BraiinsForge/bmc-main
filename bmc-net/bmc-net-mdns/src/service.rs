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

//! Advertisement lifecycle around [`crate::MdnsAdvertiser`]:
//! it starts the responder with identity data gathered at boot, follows
//! hostname changes, and sends the goodbye packets on shutdown so the device
//! drops out of browse lists promptly.
//!
//! The hostname comes from the platform [`NetworkConfig`], which signals
//! [`NetworkConfig::hostname_change_notifier`] after every hostname write,
//! whichever API applied it, so the responder renames without polling. An edit
//! made behind the network manager's back is only picked up when the responder
//! next restarts. Name conflicts are handled by the advertiser at announce time
//! only; see the crate docs.
//!
//! Enablement is the application's business: the caller drives it through a
//! [`watch`] channel, and this module only reconciles the responder to it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bmc_net::NetworkConfig;
use bmc_net_types::MacAddr;
use rand::Rng;
use tokio::sync::{Mutex, Notify, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{Advertisement, MdnsAdvertiser, TxtValues};

/// Applying a hostname on OpenWRT restarts the network, taking all interfaces
/// (loopback included) down for ~5 s; a rename issued in that window fails.
const NETWORK_SETTLE_DELAY: Duration = Duration::from_secs(8);

const RENAME_RETRY_DELAY: Duration = Duration::from_secs(5);

const START_RETRY_DELAY: Duration = Duration::from_secs(5);

const START_ATTEMPTS: usize = 3;

/// On BMM the hostname read hits the network manager, which is down during
/// network restarts, so a `None` is transient and worth retrying.
const HOSTNAME_RETRY_DELAY: Duration = Duration::from_secs(5);

const HOSTNAME_ATTEMPTS: usize = 3;

/// Trailing MAC octets in the conflict suffix, matching the `miner-<mac>`
/// default hostname convention.
const SUFFIX_OCTETS: usize = 3;

/// One advertisement lifecycle, owned end to end by [`run`]'s task. Cancelling
/// drops the follow loop at an await point (safe: advertiser operations are
/// channel waits) while the task keeps the advertiser for the goodbye.
#[derive(Debug)]
struct Instance {
    cancel_token: CancellationToken,
    run_task: JoinHandle<()>,
}

impl Instance {
    fn start(
        network: Arc<dyn NetworkConfig>,
        advertisement: Advertisement,
        announce_delay: Duration,
    ) -> Self {
        let cancel_token = CancellationToken::new();
        let run_task = tokio::spawn(run(
            network,
            advertisement,
            announce_delay,
            cancel_token.clone(),
        ));
        Self {
            cancel_token,
            run_task,
        }
    }

    /// Resolves only once the goodbye for anything the task announced is out.
    async fn shutdown(mut self) {
        self.cancel_token.cancel();
        // `&mut` because the `Drop` impl below forbids moving the handle out.
        if let Err(error) = (&mut self.run_task).await {
            log::warn!("mDNS run task failed: {error}");
        }
    }
}

impl Drop for Instance {
    /// Dropping a `JoinHandle` detaches the task; cancel instead — unlike an
    /// abort, the task still gets to say goodbye.
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

async fn run(
    network: Arc<dyn NetworkConfig>,
    advertisement: Advertisement,
    announce_delay: Duration,
    cancel_token: CancellationToken,
) {
    let hostname = advertisement.hostname.clone();
    let hostname_changed = network.hostname_change_notifier();
    if !announce_delay.is_zero() {
        log::info!(
            "mDNS: delaying first announce by {:.1}s to de-synchronize from other devices",
            announce_delay.as_secs_f32()
        );
        if cancel_token
            .run_until_cancelled(tokio::time::sleep(announce_delay))
            .await
            .is_none()
        {
            return;
        }
    }
    let Some(mut advertiser) = start_with_retry(advertisement, &cancel_token).await else {
        return;
    };
    // The loop only borrows the advertiser, so cancellation — which drops the
    // loop future wherever it is — leaves the advertiser here for its goodbye.
    cancel_token
        .run_until_cancelled(follow_hostname(
            network.as_ref(),
            &mut advertiser,
            hostname,
            &hostname_changed,
        ))
        .await;
    if let Err(error) = advertiser.shutdown().await {
        log::warn!("mDNS shutdown failed: {error}");
    }
}

/// The backend fails transiently while early boot restarts the interfaces the
/// responder depends on, so failures are retried before giving up.
async fn start_with_retry(
    advertisement: Advertisement,
    cancel_token: &CancellationToken,
) -> Option<MdnsAdvertiser> {
    for attempt in 1..=START_ATTEMPTS {
        match MdnsAdvertiser::start(advertisement.clone()).await {
            Ok(advertiser) => {
                log::info!(
                    "mDNS: advertising web UI as {}",
                    advertiser.effective_hostname()
                );
                return Some(advertiser);
            }
            Err(error) if attempt < START_ATTEMPTS => {
                log::warn!("mDNS start failed, retrying: {error}");
            }
            Err(error) => {
                log::warn!("mDNS advertisement disabled: {error}");
                return None;
            }
        }
        cancel_token
            .run_until_cancelled(tokio::time::sleep(START_RETRY_DELAY))
            .await?;
    }
    None
}

/// Identity the application advertises; the caller supplies it because
/// port and TXT payload are application concerns.
#[derive(Debug, Clone)]
pub struct ServiceIdentity {
    /// TCP port of the application's web entry point.
    pub port: u16,
    /// TXT payload advertised with the instance.
    pub txt_values: TxtValues,
}

/// Reconciles the responder to the caller's enablement signal, keeping one
/// advertisement alive for as long as it says so.
#[derive(Debug)]
pub struct MdnsService {
    network: Arc<dyn NetworkConfig>,
    identity: ServiceIdentity,
    /// Announce jitter bound, applied only at boot; toggles never jitter.
    boot_jitter: Duration,
}

impl MdnsService {
    #[must_use]
    pub fn new(
        network: Arc<dyn NetworkConfig>,
        identity: ServiceIdentity,
        boot_jitter: Duration,
    ) -> Self {
        Self {
            network,
            identity,
            boot_jitter,
        }
    }

    /// Spawns the reconcile loop (drives the responder to match `enabled`,
    /// jittered announce at boot only) and the goodbye task (cancels and
    /// says goodbye when `shutdown` fires, latching so nothing announces
    /// afresh). Never blocks the caller.
    pub fn spawn(self, enabled: watch::Receiver<bool>, shutdown: CancellationToken) {
        let state = Arc::new(State {
            network: self.network,
            identity: self.identity,
            stopping: AtomicBool::new(false),
            instance: Mutex::new(None),
        });

        // The boot reconcile runs in a task, not inline: it gathers identity
        // over the network on BMM and the caller must not wait for that.
        let reconciler = state.clone();
        let jitter = sample_boot_jitter(self.boot_jitter);
        tokio::spawn(reconcile_loop(reconciler, enabled, jitter));

        // Send goodbye the instant the shutdown signal lands: on reboot the
        // process is killed inside the web server's drain window, so the
        // goodbye must not wait for it. Spawned even while disabled — the
        // operator may enable mDNS later.
        tokio::spawn(async move {
            shutdown.cancelled().await;
            state.stop().await;
        });
    }
}

/// Reconcile until the enablement sender is dropped, which only happens when
/// the application is tearing down.
///
/// A postponed start (hostname unreadable) is retried on a timer, because the
/// enablement signal alone would not come back to re-arm the reconcile.
async fn reconcile_loop(
    state: Arc<State>,
    mut enabled: watch::Receiver<bool>,
    boot_jitter: Duration,
) {
    let mut announce_delay = boot_jitter;
    loop {
        let wanted = *enabled.borrow_and_update();
        let postponed = state.reconcile(wanted, announce_delay).await;
        announce_delay = Duration::ZERO;
        if postponed {
            tokio::select! {
                changed = enabled.changed() => if changed.is_err() { return },
                () = tokio::time::sleep(HOSTNAME_RETRY_DELAY) => {}
            }
        } else if enabled.changed().await.is_err() {
            return;
        }
    }
}

/// Live lifecycle state, shared by the reconcile loop and the goodbye task.
#[derive(Debug)]
struct State {
    network: Arc<dyn NetworkConfig>,
    identity: ServiceIdentity,
    /// Latched by [`stop`](Self::stop) so a racing toggle cannot announce an
    /// instance that would die without its goodbye.
    stopping: AtomicBool,
    instance: Mutex<Option<Instance>>,
}

impl State {
    /// Bring the responder in line with `enabled`. Idempotent. Returns whether
    /// a wanted start had to be postponed and needs retrying.
    async fn reconcile(&self, enabled: bool, announce_delay: Duration) -> bool {
        if enabled {
            self.ensure_running(announce_delay).await
        } else {
            self.shutdown().await;
            false
        }
    }

    /// Terminal shutdown, driven solely by the token passed to
    /// [`MdnsService::spawn`]; unlike the disable path it latches
    /// [`stopping`](Self::stopping) so nothing announces afresh.
    async fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        self.shutdown().await;
    }

    /// The slot stays locked until the goodbye is out, so a disable/enable
    /// sequence cannot announce a fresh instance while the previous one's
    /// TTL=0 records are still in flight and would evict it from peer caches.
    async fn shutdown(&self) {
        let mut slot = self.instance.lock().await;
        if let Some(instance) = slot.take() {
            instance.shutdown().await;
        }
    }

    async fn ensure_running(&self, announce_delay: Duration) -> bool {
        // Fetched before taking the slot lock: a stalled fetch must not make
        // `stop` wait for the lock and hold up the whole process shutdown.
        let Some(hostname) = read_hostname(self.network.as_ref()).await else {
            // Stay stopped and retry later; advertising a fallback name would
            // stick until the next rename.
            log::warn!("mDNS start postponed: no hostname available");
            return true;
        };
        let mut slot = self.instance.lock().await;
        // Checked under the slot lock so no reconcile slips a new instance in
        // after `stop`'s goodbye.
        if self.stopping.load(Ordering::Acquire) || slot.is_some() {
            return false;
        }
        let advertisement = Advertisement {
            hostname,
            conflict_suffix: suffix_from_mac(self.network.eth_data().mac),
            port: self.identity.port,
            txt_values: self.identity.txt_values.clone(),
        };
        *slot = Some(Instance::start(
            self.network.clone(),
            advertisement,
            announce_delay,
        ));
        false
    }
}

/// Follow hostname changes, renaming the advertised instance. Runs until the
/// caller cancels it.
async fn follow_hostname(
    network: &dyn NetworkConfig,
    advertiser: &mut MdnsAdvertiser,
    mut advertised: String,
    hostname_changed: &Notify,
) {
    loop {
        hostname_changed.notified().await;
        advertised = rename_to_current(network, advertiser, advertised).await;
    }
}

/// Rename the instance to the current hostname, retrying on an interval until
/// the advertised name matches; returns the name that ended up advertised.
/// Re-reading the hostname per tick means one changed again mid-retry simply
/// becomes the new target instead of a stale rename.
async fn rename_to_current(
    network: &dyn NetworkConfig,
    advertiser: &mut MdnsAdvertiser,
    advertised: String,
) -> String {
    // The first tick lands after the settle delay: applying a hostname
    // restarts the network out from under the responder.
    let mut retry = tokio::time::interval_at(
        tokio::time::Instant::now() + NETWORK_SETTLE_DELAY,
        RENAME_RETRY_DELAY,
    );
    loop {
        retry.tick().await;
        let Some(hostname) = read_hostname(network).await else {
            log::warn!("mDNS rename: hostname read failed, retrying");
            continue;
        };
        if hostname == advertised {
            return advertised;
        }
        match advertiser.rename(hostname.clone()).await {
            Ok(()) => return hostname,
            Err(error) => log::warn!("mDNS rename to '{hostname}' failed, retrying: {error}"),
        }
    }
}

/// Read the hostname, retrying transient failures.
async fn read_hostname(network: &dyn NetworkConfig) -> Option<String> {
    for attempt in 1..=HOSTNAME_ATTEMPTS {
        if let Some(hostname) = network.hostname().await {
            return Some(hostname);
        }
        log::warn!("mDNS: hostname read failed (attempt {attempt}/{HOSTNAME_ATTEMPTS})");
        if attempt < HOSTNAME_ATTEMPTS {
            tokio::time::sleep(HOSTNAME_RETRY_DELAY).await;
        }
    }
    None
}

/// Inclusive range: a zero bound must sample, not panic.
fn sample_boot_jitter(bound: Duration) -> Duration {
    rand::rng().random_range(Duration::ZERO..=bound)
}

/// Conflict suffix from the trailing MAC octets — the same tail the platforms
/// derive their `miner-<mac>` default hostname from, so a fallback name keeps
/// the shape operators already recognise.
fn suffix_from_mac(mac: Option<MacAddr>) -> String {
    let Some(mac) = mac else {
        return String::new();
    };
    let text = mac.to_string();
    let octets: Vec<&str> = text.split(MacAddr::DELIMITER).collect();
    octets[octets.len().saturating_sub(SUFFIX_OCTETS)..].concat()
}
