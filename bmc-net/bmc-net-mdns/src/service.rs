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
//! One task owns the advertiser as a local and converges it on the desired
//! state one bounded step per wake-up; toggles, hostname notifications and
//! retry deadlines are nothing but wake-ups. With no waits inside the
//! reconcile step, no advertiser operation is ever cancelled mid-flight, and
//! starts, renames and goodbyes are strictly sequential — a fresh announce
//! can never overlap a goodbye whose TTL=0 records would evict it from peer
//! caches.

use std::sync::Arc;
use std::time::Duration;

use bmc_net::NetworkConfig;
use bmc_net_types::MacAddr;
use rand::Rng;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::{Advertisement, MdnsAdvertiser, TxtValues};

/// Applying a hostname on OpenWRT restarts the network, taking all interfaces
/// (loopback included) down for ~5 s; a rename issued in that window fails.
const NETWORK_SETTLE_DELAY: Duration = Duration::from_secs(8);

/// One cadence for every transient failure: hostname unreadable (on BMM the
/// read hits the network manager, which is down during network restarts),
/// responder start failed, rename failed.
const RETRY_DELAY: Duration = Duration::from_secs(5);

/// The hostname read runs inside the reconcile step, ahead of the select
/// that watches the shutdown signal, so a wedged hostname source (on
/// OpenWRT an uci subprocess that can block on a lock) must be cut short
/// or it would hold up the shutdown goodbye indefinitely.
const HOSTNAME_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Trailing MAC octets in the conflict suffix, matching the `miner-<mac>`
/// default hostname convention.
const SUFFIX_OCTETS: usize = 3;

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
    /// Announce jitter bound. The window is anchored at task start: an
    /// enable landing inside it waits out the remainder, toggles after it
    /// never jitter.
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

    /// The whole lifecycle lives in one spawned task so the caller is never
    /// blocked — identity is gathered over the network on some platforms.
    /// `enabled` carries the desired state as a value, so a missed
    /// notification costs nothing; `shutdown` drives the goodbye directly
    /// because on reboot the process is killed inside the web server's drain
    /// window and the goodbye must not wait for it.
    ///
    /// The goodbye is emitted only after cancellation, on this task — so a
    /// caller that lets its runtime drop right after cancelling kills the task
    /// mid-goodbye and leaves peers with a stale advertisement until its
    /// records age out. Await the returned handle to order the packets before
    /// process exit; bound the wait, as the goodbye path itself waits on the
    /// backend for up to a few seconds (see the crate's shutdown timeouts) and
    /// a reconcile step in flight can add its own hostname read on top.
    #[must_use]
    pub fn spawn(
        self,
        enabled: watch::Receiver<bool>,
        shutdown: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(self.run(enabled, shutdown))
    }

    /// A dropped enablement sender means the application is tearing down, so
    /// it exits like a shutdown, goodbye included.
    async fn run(self, mut enabled: watch::Receiver<bool>, shutdown: CancellationToken) {
        let hostname_changed = self.network.hostname_change_notifier();
        let jitter = sample_boot_jitter(self.boot_jitter);
        if !jitter.is_zero() {
            log::info!(
                "mDNS: delaying first announce by {:.1}s to de-synchronize from other devices",
                jitter.as_secs_f32()
            );
        }
        // The earliest permitted announce or rename; wake-ups landing sooner
        // reschedule instead of acting. Carries the boot jitter now and the
        // settle window after each hostname notification.
        let mut not_before = Instant::now() + jitter;
        let mut running: Option<(MdnsAdvertiser, String)> = None;
        loop {
            let wanted = *enabled.borrow_and_update();
            let deadline = self.reconcile(&mut running, wanted, not_before).await;
            tokio::select! {
                () = shutdown.cancelled() => break,
                changed = enabled.changed() => if changed.is_err() { break },
                () = hostname_changed.notified() => {
                    not_before = Instant::now() + NETWORK_SETTLE_DELAY;
                }
                // The expression is evaluated even when disabled, hence the
                // fallback; it is never polled without a deadline.
                () = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now)),
                    if deadline.is_some() => {}
            }
        }
        if let Some((advertiser, _)) = running.take() {
            say_goodbye(advertiser).await;
        }
    }

    /// One bounded convergence step: no waits, only the advertiser and
    /// hostname operations themselves, so the caller's select never cancels
    /// work in flight. Returns when to wake again if the step could not
    /// finish the job.
    async fn reconcile(
        &self,
        running: &mut Option<(MdnsAdvertiser, String)>,
        wanted: bool,
        not_before: Instant,
    ) -> Option<Instant> {
        if !wanted {
            if let Some((advertiser, _)) = running.take() {
                say_goodbye(advertiser).await;
            }
            return None;
        }
        if Instant::now() < not_before {
            return Some(not_before);
        }
        let read = tokio::time::timeout(HOSTNAME_READ_TIMEOUT, self.network.hostname()).await;
        let Ok(Some(hostname)) = read else {
            // Stay as we are and retry; announcing a fallback name would
            // stick until the next rename.
            log::warn!("mDNS: hostname unavailable, retrying");
            return Some(Instant::now() + RETRY_DELAY);
        };
        match running.as_mut() {
            None => {
                let advertisement = Advertisement {
                    hostname: hostname.clone(),
                    conflict_suffix: suffix_from_mac(self.network.eth_data().mac),
                    port: self.identity.port,
                    txt_values: self.identity.txt_values.clone(),
                };
                match MdnsAdvertiser::start(advertisement).await {
                    Ok(advertiser) => {
                        log::info!(
                            "mDNS: advertising web UI as {}",
                            advertiser.effective_hostname()
                        );
                        *running = Some((advertiser, hostname));
                        None
                    }
                    Err(error) => {
                        log::warn!("mDNS start failed, retrying: {error}");
                        Some(Instant::now() + RETRY_DELAY)
                    }
                }
            }
            Some((advertiser, advertised)) => {
                if hostname == *advertised {
                    return None;
                }
                match advertiser.rename(hostname.clone()).await {
                    Ok(()) => {
                        *advertised = hostname;
                        None
                    }
                    Err(error) => {
                        log::warn!("mDNS rename to '{hostname}' failed, retrying: {error}");
                        Some(Instant::now() + RETRY_DELAY)
                    }
                }
            }
        }
    }
}

async fn say_goodbye(advertiser: MdnsAdvertiser) {
    if let Err(error) = advertiser.shutdown().await {
        log::warn!("mDNS shutdown failed: {error}");
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Platforms that default mDNS on use a zero jitter bound, so the
    /// degenerate range must sample rather than panic.
    #[test]
    fn zero_boot_jitter_samples_zero() {
        for _ in 0..100 {
            assert_eq!(sample_boot_jitter(Duration::ZERO), Duration::ZERO);
        }
    }

    /// The device measured for BOS-4004 reports MAC `1a:94:fe:18:ba:7a` and a
    /// default hostname of `miner-18ba7a`, so the conflict suffix has to come
    /// out as the same `18ba7a` the platform already derived.
    #[test]
    fn suffix_matches_the_platform_hostname_convention() {
        let mac = MacAddr::from([0x1a, 0x94, 0xfe, 0x18, 0xba, 0x7a]);
        assert_eq!(suffix_from_mac(Some(mac)), "18ba7a");
    }

    #[test]
    fn suffix_pads_single_digit_octets() {
        let mac = MacAddr::from([0, 0, 0, 0x01, 0x02, 0x03]);
        assert_eq!(suffix_from_mac(Some(mac)), "010203");
    }

    #[test]
    fn missing_mac_yields_no_suffix() {
        assert_eq!(suffix_from_mac(None), "");
    }

    /// The task must be joinable, or a caller has no way to order the goodbye
    /// before process exit. Kept disabled so the assertion is about the handle
    /// alone: with an advertisement actually on the air the join would depend
    /// on a real responder and on link timing.
    #[tokio::test]
    async fn awaiting_the_handle_observes_the_task_finishing() {
        let network = Arc::new(bmc_net::mock::MockNetworkManager::default());
        let shutdown = CancellationToken::new();
        let (_enabled_tx, enabled) = watch::channel(false);
        let handle = MdnsService::new(
            network,
            ServiceIdentity {
                port: 80,
                txt_values: TxtValues::default(),
            },
            Duration::ZERO,
        )
        .spawn(enabled, shutdown.clone());
        shutdown.cancel();
        handle.await.expect("BUG: mDNS task must not panic");
    }
}
