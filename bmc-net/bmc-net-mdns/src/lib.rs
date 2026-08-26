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

//! mDNS/DNS-SD advertisement of the device web UI and API (BOS-4004).
//!
//! The device is advertised as a single `_http._tcp` service instance tagged
//! with the `_bos` subtype so that BOS-aware browsers can filter for it:
//!
//! ```text
//! _http._tcp.local.            PTR  <name>._http._tcp.local.
//! _bos._sub._http._tcp.local.  PTR  <name>._http._tcp.local.
//! <name>._http._tcp.local.     SRV  0 0 <port> <name>.local.
//! <name>._http._tcp.local.     TXT  hostname=… bos_version=… …
//! <name>.local.                A/AAAA  <per-interface address>
//! ```
//!
//! `<name>` is the configured hostname, used as-is. The name is probed at
//! announce time: when the backend's RFC 6762 §9 probing reports within the
//! probe-verdict window that the name is already taken on the link, the
//! advertiser re-registers as `<hostname>-<suffix>`, where the suffix is the
//! caller-supplied device-unique tail (see [`Advertisement::conflict_suffix`]);
//! a conflict on the suffixed name is left to the backend. Conflicts
//! after the window (a twin joining the link later) are deliberately not
//! handled at all; the cost is two devices being hard to tell apart in a
//! browse list. The advertised name is a discovery label, not a stable device
//! identifier (RFC 6762 §3).
//!
//! The responder publishes on every up, non-loopback interface, IPv4 and
//! IPv6 alike, and answers with the addresses valid on the interface the
//! query came from.
//!
//! On [`MdnsAdvertiser::shutdown`] and hostname rename, goodbye packets (TTL=0)
//! are emitted for the retired instance so peers drop the stale entry within
//! seconds instead of waiting out record TTLs. A rename withdraws the old
//! instance before registering the new one: the goodbye is only emitted with
//! its records intact while the instance is in a steady announced state, so
//! the two operations must not overlap. Goodbye failures are logged rather
//! than propagated.
//!
//! One goodbye defect is inherited from the backend: when its probing
//! auto-renames a record (`foo` → `foo (2)`), unregister still builds the TTL=0
//! records from the original names. Withdrawing a name we lost the probe for
//! therefore says goodbye on behalf of the device that won it, and since the
//! PTR, SRV and TXT match that device's own records byte for byte, peers drop
//! it from browse lists until it announces again. Fixed upstream by
//! <https://github.com/keepsimple1/mdns-sd/pull/495>, unreleased as of
//! mdns-sd 0.21.0; the dependency bump is tracked by BOS-4042.
//!
//! The async methods only ever await the backend's event channels — sending a
//! command to the responder daemon is a non-blocking channel write — so they
//! are safe on a runtime thread and safe to drop at an await point: the
//! daemon keeps its own state, and an abandoned wait only loses the
//! confirmation, never the command.
//!
//! Every operation here is best-effort and may fail transiently while the
//! network is being reconfigured, because the backend needs a loopback socket
//! to reach its own daemon thread and needs interface addresses to send from.
//! Neither exists during a network restart — which on OpenWRT is exactly what
//! applying a new hostname does. Callers must therefore retry a failed start
//! or rename rather than treat it as terminal; see
//! [`MdnsAdvertiser::rename`].

pub mod name;
pub mod service;

use std::fmt;
use std::time::Duration;

use mdns_sd::{
    DaemonEvent, IfKind, Receiver, RecvError, ServiceDaemon, ServiceInfo, UnregisterStatus,
};
use tokio::time::timeout;

/// Registering under the subtype also registers the base `_http._tcp` type.
pub const BOS_SUBTYPE: &str = "_bos._sub._http._tcp.local.";

/// Bounds on waiting for the daemon to confirm an operation.
///
/// Confirmations arrive after one trip through the daemon's event loop —
/// mdns-sd 0.21.0 sends the first goodbye inline before replying (see its
/// `exec_command_unregister` and `cleanup`) — so neither value covers the
/// resend, only scheduling margin. Waiting for the resend is
/// [`GOODBYE_RESEND_WINDOW`], paid separately. Unregister confirms a single
/// service; shutdown also says goodbye per service and tears down every
/// socket, so it gets the larger margin.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const UNREGISTER_TIMEOUT: Duration = Duration::from_millis(500);

/// How long to keep the daemon alive after a withdraw so its own goodbye
/// resend goes out.
///
/// mdns-sd 0.21.0 queues a second goodbye 120 ms after `unregister`
/// ("repeat for one time just in case some peers miss the message"), but its
/// `Exit` path emits one goodbye per service and then returns from the event
/// loop, discarding queued retransmissions. Unregistering first and waiting
/// out that timer buys the second packet: a goodbye is unacknowledged
/// multicast, and the one that gets lost is what leaves a peer showing a
/// miner that is no longer there.
const GOODBYE_RESEND_WINDOW: Duration = Duration::from_millis(200);

/// How long [`MdnsAdvertiser::start`] waits for probing to report a conflict.
///
/// RFC 6762 §8.1 probes three times at 250 ms intervals, so a verdict is in
/// well under a second; the margin covers a loaded link. This is a worst-case
/// bound only — the wait ends as soon as the backend announces the instance
/// (name confirmed free) or reports a rename (conflict). Waiting for the
/// verdict means the caller never sees a name that changes moments later.
const PROBE_VERDICT_TIMEOUT: Duration = Duration::from_millis(1500);

/// The backend is an implementation detail; callers should treat these errors
/// as opaque beyond logging.
#[derive(Debug)]
pub enum Error {
    Backend(mdns_sd::Error),
    ShutdownTimeout,
    ShutdownConfirmationLost,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(error) => write!(f, "mDNS backend: {error}"),
            Self::ShutdownTimeout => write!(f, "mDNS daemon did not confirm shutdown in time"),
            Self::ShutdownConfirmationLost => {
                write!(f, "mDNS daemon exited before confirming shutdown")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Backend(error) => Some(error),
            Self::ShutdownTimeout | Self::ShutdownConfirmationLost => None,
        }
    }
}

impl From<mdns_sd::Error> for Error {
    fn from(error: mdns_sd::Error) -> Self {
        Self::Backend(error)
    }
}

/// Everything the device wants to say about itself.
///
/// The device advertises under `hostname` as-is. The clean hostname is also
/// repeated in TXT under `hostname=` so browsers can display it even when a
/// conflict has forced a different advertised name.
#[derive(Debug, Clone)]
pub struct Advertisement {
    /// Configured hostname, advertised verbatim while it is free.
    pub hostname: String,
    /// Device-unique tail appended to the hostname, but only after the link
    /// reports the plain hostname is taken.
    pub conflict_suffix: String,
    /// TCP port of the device's web entry point.
    pub port: u16,
    /// TXT payload advertised with the instance.
    pub txt_values: TxtValues,
}

/// TXT payload values, all sourced from the live system.
///
/// `bos_version` identifies the device as BOS;
/// when it is absent the advertiser emits the minimum accepted fallback
/// `bos=1` instead, so the payload always distinguishes BOS from other
/// `_http._tcp` devices.
#[derive(Debug, Clone, Default)]
pub struct TxtValues {
    /// Full BOS version string, e.g. `2026-07-07-0-c5a2978a-26.07-plus`.
    pub bos_version: Option<String>,
    /// Exact BOS API version implemented by the device, e.g. `1.6.0`.
    pub bos_api_version: Option<String>,
    /// Human-readable device model, e.g. `Braiins Mini Miner BMM101`.
    pub miner: Option<String>,
}

/// Handle owning one registered service instance.
///
/// The backend daemon has no teardown on drop: a dropped handle leaves the
/// responder thread running and the instance advertised. Always end with
/// [`shutdown`](Self::shutdown), which also emits the goodbyes.
pub struct MdnsAdvertiser {
    daemon: ServiceDaemon,
    advertisement: Advertisement,
    effective_hostname: String,
    /// Instance currently on the air; `None` between a withdraw and the
    /// registration of its replacement (a failed rename parks here).
    fullname: Option<String>,
}

impl fmt::Debug for MdnsAdvertiser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MdnsAdvertiser")
            .field("fullname", &self.fullname)
            .field("advertisement", &self.advertisement)
            .finish_non_exhaustive()
    }
}

impl MdnsAdvertiser {
    /// One start covers the process lifetime of an advertisement: the
    /// responder binds every up, non-loopback interface (IPv4 and IPv6) and
    /// tracks addresses on its own, so callers never restart it for
    /// interface churn — they come back only for [`rename`](Self::rename)
    /// and [`shutdown`](Self::shutdown).
    pub async fn start(advertisement: Advertisement) -> Result<Self, Error> {
        let daemon = ServiceDaemon::new()?;
        let mut advertiser = Self {
            daemon,
            advertisement,
            effective_hostname: String::new(),
            fullname: None,
        };
        let announced = async {
            advertiser
                .daemon
                .disable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])?;
            advertiser.announce().await
        }
        .await;
        if let Err(error) = announced {
            // The backend has no teardown on drop, so without this a failed
            // start would leave the daemon thread running detached — and a
            // retrying caller would stack up another one per attempt.
            if let Err(shutdown_error) = advertiser.daemon.shutdown() {
                log::warn!(
                    "mDNS: could not stop the daemon after a failed start: {shutdown_error}"
                );
            }
            return Err(error);
        }
        Ok(advertiser)
    }

    /// The single path onto the air, shared by start and rename: candidate
    /// names are published in order of preference and the first that survives
    /// its probe window — or the last one, taken or not — stays registered.
    ///
    /// The bare hostname always comes first, even when renaming away
    /// from a contested name: an old suffix is never carried along.
    /// The suffixed fallback is a candidate only when the suffix actually
    /// changes the name, so "no fallback available" is a one-element list
    /// rather than a special case. A conflict on the final candidate is
    /// left to the backend's own rename.
    async fn announce(&mut self) -> Result<(), Error> {
        let bare = name::effective_hostname(&self.advertisement.hostname, "");
        let suffixed = name::effective_hostname(
            &self.advertisement.hostname,
            &self.advertisement.conflict_suffix,
        );
        let fallback = (suffixed != bare).then_some(suffixed);
        for candidate in std::iter::once(bare).chain(fallback) {
            if !self.publish(&candidate).await? {
                break;
            }
        }
        log::info!("mDNS: advertising as '{}'", self.effective_hostname);
        Ok(())
    }

    /// One publish attempt: withdraw whatever is on the air, register
    /// `hostname`, and report whether probing said the name is taken
    /// on the link. Withdraw comes first so the goodbye is emitted while
    /// the old instance is still in a steady, announced state; a goodbye
    /// raced by its own replacement can evict the fresh records from
    /// peer caches.
    ///
    /// On error the old instance has already been withdrawn and nothing is
    /// advertised — deliberately: re-registering the old name would race the
    /// queued withdraw and resurrect an instance that is being retired.
    /// Callers are expected to retry the whole announce.
    async fn publish(&mut self, hostname: &str) -> Result<bool, Error> {
        if let Some(retired) = self.fullname.take() {
            withdraw(&self.daemon, &retired).await;
        }
        // Subscribed before registering so the probe verdict for this name
        // cannot be missed: the backend's event channels are bounded and drop
        // events once full, so only a fresh channel is guaranteed to still
        // hold this name's verdict — and cannot hold a stale verdict for a
        // previous name.
        let monitor = self.daemon.monitor()?;
        let fullname = register(&self.daemon, &self.advertisement, hostname)?;
        hostname.clone_into(&mut self.effective_hostname);
        let taken = name_taken(&monitor, &fullname, hostname).await;
        self.fullname = Some(fullname);
        Ok(taken)
    }

    #[must_use]
    pub fn effective_hostname(&self) -> &str {
        &self.effective_hostname
    }

    /// Every rename is a full goodbye-and-replace, even when the effective
    /// name comes out unchanged (distinct hostnames can share one effective
    /// name — slugify collapses separators): the replacement is what keeps
    /// the TXT `hostname` value current, and treating every rename the same
    /// keeps retries after a failure trivially idempotent.
    ///
    /// On error the old instance has already been withdrawn and nothing is
    /// advertised (see [`publish`](Self::publish)); retrying the call simply
    /// announces the replacement from scratch. Callers are expected to retry.
    pub async fn rename(&mut self, hostname: impl Into<String>) -> Result<(), Error> {
        let previous = self.effective_hostname.clone();
        self.advertisement.hostname = hostname.into();
        self.announce().await?;
        log::info!("mDNS: renamed {previous} -> {}", self.effective_hostname);
        Ok(())
    }

    /// The goodbye (TTL=0) races process death and network teardown, so this
    /// waits for the daemon's confirmation — callers can then order it
    /// strictly before taking the network down instead of hoping the packets
    /// made it out.
    ///
    /// Withdrawing before the daemon exits is what gets the goodbye sent
    /// twice; see [`GOODBYE_RESEND_WINDOW`].
    pub async fn shutdown(self) -> Result<(), Error> {
        if let Some(fullname) = self.fullname.as_deref() {
            withdraw(&self.daemon, fullname).await;
            tokio::time::sleep(GOODBYE_RESEND_WINDOW).await;
        }
        let receiver = self.daemon.shutdown()?;
        timeout(SHUTDOWN_TIMEOUT, receiver.recv_async())
            .await
            .map_err(|_| Error::ShutdownTimeout)?
            .map_err(|_| Error::ShutdownConfirmationLost)?;
        log::info!(
            "mDNS: said goodbye as {}",
            self.fullname.as_deref().unwrap_or("<nothing advertised>")
        );
        Ok(())
    }
}

/// Probing succeeds by silence (RFC 6762 §8.1): there is no positive
/// "name is free" packet, so the answer is `true` only when the backend
/// reports a conflict before the window closes — anything else, including
/// the daemon dying, defaults to keeping the name.
async fn name_taken(
    monitor: &Receiver<DaemonEvent>,
    fullname: &str,
    effective_hostname: &str,
) -> bool {
    let window = tokio::time::sleep(PROBE_VERDICT_TIMEOUT);
    tokio::pin!(window);
    loop {
        let event = tokio::select! {
            // Silence is a verdict: probing had its window and reported
            // no conflict.
            () = &mut window => return false,
            received = monitor.recv_async() => match received {
                Ok(event) => event,
                // The daemon is gone; nothing more will be reported and
                // the failure surfaces on the next daemon call.
                Err(RecvError::Disconnected) => return false,
            },
        };
        if let Some(taken) = name_taken_from(event, fullname, effective_hostname) {
            return taken;
        }
    }
}

/// The monitor channel is a firehose shared with interface changes and
/// foreign instances, so most events say nothing about our name; `None`
/// keeps the wait going rather than letting the first event decide.
#[expect(clippy::wildcard_enum_match_arm)]
fn name_taken_from(event: DaemonEvent, fullname: &str, effective_hostname: &str) -> Option<bool> {
    match event {
        DaemonEvent::NameChange(change)
            if owns_name(fullname, effective_hostname, &change.original) =>
        {
            log::info!(
                "mDNS: '{effective_hostname}' is taken on this link (backend chose '{}')",
                change.new_name
            );
            Some(true)
        }
        // Our own announce only goes out once probing succeeded on an
        // interface — a positive verdict ahead of the window.
        DaemonEvent::Announce(announced, _) if announced == fullname => Some(false),
        DaemonEvent::Error(error) => {
            log::debug!("mDNS: daemon reported an error during the probe window: {error}");
            None
        }
        // Foreign instances' renames and announces, interface changes
        // and query responses say nothing about our name — and neither
        // can an event added by a future backend (the enum is
        // `#[non_exhaustive]`, hence wildcard).
        _ => None,
    }
}

/// A conflict can be reported against either the service instance record
/// (`fullname`) or the host record (`effective_hostname` under `.local.`),
/// and the two differ in shape, so both spellings must count as ours.
fn owns_name(fullname: &str, effective_hostname: &str, original: &str) -> bool {
    original == fullname || original.strip_suffix(".local.") == Some(effective_hostname)
}

/// Failures are logged and never propagated: the caller is mid-rename and must
/// go on to register the replacement, and a transient backend error can be
/// reported even after the command was queued.
async fn withdraw(daemon: &ServiceDaemon, fullname: &str) {
    match daemon.unregister(fullname) {
        Ok(receiver) => match timeout(UNREGISTER_TIMEOUT, receiver.recv_async()).await {
            Ok(Ok(UnregisterStatus::OK)) => {}
            Ok(Ok(UnregisterStatus::NotFound)) => {
                log::warn!("mDNS: {fullname} not found for goodbye");
            }
            // A closed confirmation channel is the daemon exiting without
            // confirming — the shape a shutdown race takes — not a slow one.
            Ok(Err(_)) => {
                log::warn!("mDNS: daemon exited before confirming goodbye for {fullname}");
            }
            Err(_) => log::warn!("mDNS: goodbye for {fullname} timed out"),
        },
        Err(error) => log::warn!("mDNS: unregister {fullname} failed: {error}"),
    }
}

fn register(
    daemon: &ServiceDaemon,
    advertisement: &Advertisement,
    effective_hostname: &str,
) -> Result<String, Error> {
    let mut txt_values: Vec<(&str, &str)> = vec![("hostname", advertisement.hostname.as_str())];
    match &advertisement.txt_values.bos_version {
        Some(version) => txt_values.push(("bos_version", version)),
        None => txt_values.push(("bos", "1")),
    }
    if let Some(api_version) = &advertisement.txt_values.bos_api_version {
        txt_values.push(("bos_api_version", api_version));
    }
    if let Some(miner) = &advertisement.txt_values.miner {
        txt_values.push(("miner", miner));
    }

    let info = ServiceInfo::new(
        BOS_SUBTYPE,
        effective_hostname,
        &format!("{effective_hostname}.local."),
        (),
        advertisement.port,
        &txt_values[..],
    )?
    .enable_addr_auto();

    let fullname = info.get_fullname().to_owned();
    daemon.register(info)?;
    Ok(fullname)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULLNAME: &str = "miner-a._http._tcp.local.";
    const EFFECTIVE: &str = "miner-a";

    #[test]
    fn owns_matches_the_instance_and_host_records() {
        assert!(owns_name(FULLNAME, EFFECTIVE, FULLNAME));
        assert!(owns_name(FULLNAME, EFFECTIVE, "miner-a.local."));
    }

    #[test]
    fn owns_rejects_other_names() {
        assert!(!owns_name(FULLNAME, EFFECTIVE, "miner-b._http._tcp.local."));
        assert!(!owns_name(FULLNAME, EFFECTIVE, "miner-b.local."));
        // A report for the name this handle was renamed away from must not
        // pass once the recorded names have moved on.
        assert!(!owns_name("miner-b._http._tcp.local.", "miner-b", FULLNAME));
        // The host record must be fully qualified.
        assert!(!owns_name(FULLNAME, EFFECTIVE, "miner-a.local"));
        assert!(!owns_name(FULLNAME, EFFECTIVE, "miner-a"));
    }
}
