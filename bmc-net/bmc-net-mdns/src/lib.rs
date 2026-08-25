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
//! `<name>` is the configured hostname, used as-is. The name is probed once,
//! at announce time: when the backend's RFC 6762 §9 probing reports within
//! the probe-verdict window that the name is already taken on the link, the
//! advertiser re-registers as `<hostname>-<suffix>`, where the suffix is the
//! caller-supplied device-unique tail (see [`Advertisement::conflict_suffix`])
//! — no questions asked, the suffixed name is not probed again. Conflicts
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
//! One goodbye gap is inherited from the backend: when the backend's probing
//! auto-renames a record (`foo` → `foo (2)`), its unregister still builds the
//! TTL=0 records from the original names, so peers that cached the renamed
//! records only age them out. The escalation below replaces such an instance
//! within the probe-verdict window, which keeps the stale entry's lifetime
//! bounded by the record TTL rather than fixable here.
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

use mdns_sd::{DaemonEvent, IfKind, Receiver, ServiceDaemon, ServiceInfo, UnregisterStatus};
use tokio::time::{Instant, timeout, timeout_at};

/// Registering under the subtype also registers the base `_http._tcp` type.
pub const BOS_SUBTYPE: &str = "_bos._sub._http._tcp.local.";

/// Bounds on waiting for the daemon to confirm an operation.
///
/// Confirmations arrive after one trip through the daemon's event loop —
/// mdns-sd 0.21.0 sends the goodbyes inline before replying (see its
/// `exec_command_unregister` and `cleanup`) — so neither value carries
/// protocol time, only scheduling margin. Unregister confirms a single
/// service; shutdown also says goodbye per service and tears down every
/// socket, so it gets the larger margin.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const UNREGISTER_TIMEOUT: Duration = Duration::from_millis(500);

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
/// `bos_version` identifies the device as BOS per the BOS-4004 contract;
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
    fullname: String,
    /// Whether the names recorded above are actually registered with the
    /// daemon. Cleared before a withdraw and set again only after the
    /// replacement registration went through, so a failed rename cannot
    /// leave the handle claiming an instance that is no longer advertised.
    registered: bool,
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
    /// Start the responder and announce the service on all up, non-loopback
    /// interfaces, IPv4 and IPv6. Addresses are tracked automatically as
    /// interfaces come and go.
    pub async fn start(advertisement: Advertisement) -> Result<Self, Error> {
        let daemon = ServiceDaemon::new()?;
        let started = Self::register_and_probe(daemon.clone(), advertisement).await;
        if started.is_err() {
            // The backend has no teardown on drop, so without this a failed
            // start would leave the daemon thread running detached — and a
            // retrying caller would stack up another one per attempt.
            if let Err(error) = daemon.shutdown() {
                log::warn!("mDNS: could not stop the daemon after a failed start: {error}");
            }
        }
        started
    }

    async fn register_and_probe(
        daemon: ServiceDaemon,
        advertisement: Advertisement,
    ) -> Result<Self, Error> {
        daemon.disable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])?;
        // Subscribed before registering so the probe verdict for the very
        // first name cannot be missed.
        let monitor = daemon.monitor()?;

        let effective_hostname = name::effective_hostname(&advertisement.hostname, "");
        let fullname = register(&daemon, &advertisement, &effective_hostname)?;
        log::info!("mDNS: advertising {fullname}");

        let mut advertiser = Self {
            daemon,
            advertisement,
            effective_hostname,
            fullname,
            registered: true,
        };
        advertiser.probe_verdict(&monitor).await?;
        Ok(advertiser)
    }

    /// Give probing a bounded window to report that the plain hostname is
    /// taken, so a conflicting name is replaced before the caller starts
    /// telling users what the device is called. This is the only conflict
    /// handling there is: a name that survives its probe window is kept for
    /// the lifetime of the registration.
    ///
    /// The common case returns early: the backend only announces the instance
    /// under our own fullname once probing succeeded on an interface, so that
    /// announce is a positive verdict and the window need not be waited out.
    ///
    /// `monitor` must be subscribed just before the name was registered: the
    /// backend's event channels are bounded and drop events once full, so
    /// only a fresh channel is guaranteed to still hold this name's verdict —
    /// and cannot hold a stale verdict for a previous name.
    async fn probe_verdict(&mut self, monitor: &Receiver<DaemonEvent>) -> Result<(), Error> {
        let deadline = Instant::now() + PROBE_VERDICT_TIMEOUT;
        // Wait out other events (address changes, announces of renamed or
        // foreign instances) rather than taking the first one as the verdict.
        loop {
            match timeout_at(deadline, monitor.recv_async()).await {
                Ok(Ok(DaemonEvent::NameChange(change)))
                    if owns_name(&self.fullname, &self.effective_hostname, &change.original) =>
                {
                    return self.escalate(&change.new_name).await;
                }
                Ok(Ok(DaemonEvent::Announce(fullname, _))) if fullname == self.fullname => {
                    return Ok(());
                }
                Ok(Ok(_)) => {}
                // No verdict inside the window (or the daemon is gone); the
                // name is treated as free.
                Ok(Err(_)) | Err(_) => return Ok(()),
            }
        }
    }

    /// Re-register under `<hostname>-<conflict_suffix>`, without probing the
    /// suffixed name again: the suffix is device-unique, so a collision on it
    /// is not worth defending against and is left to the backend.
    ///
    /// `backend_name` is the name the backend picked on its own (`foo (2)` /
    /// `foo-2`); it is only logged.
    async fn escalate(&mut self, backend_name: &str) -> Result<(), Error> {
        let effective_hostname = name::effective_hostname(
            &self.advertisement.hostname,
            &self.advertisement.conflict_suffix,
        );
        if effective_hostname == self.effective_hostname {
            // No suffix to fall back to (or it does not change the name);
            // the backend's own rename is all the resolution there is.
            log::warn!(
                "mDNS: '{}' is taken on this link and no conflict suffix is \
                 available, leaving the backend's '{backend_name}'",
                self.effective_hostname
            );
            return Ok(());
        }

        log::info!(
            "mDNS: '{}' is taken on this link (backend chose '{backend_name}'), \
             advertising as '{effective_hostname}' instead",
            self.effective_hostname
        );
        self.registered = false;
        withdraw(&self.daemon, &self.fullname).await;
        self.fullname = register(&self.daemon, &self.advertisement, &effective_hostname)?;
        self.registered = true;
        self.effective_hostname = effective_hostname;
        Ok(())
    }

    #[must_use]
    pub fn effective_hostname(&self) -> &str {
        &self.effective_hostname
    }

    /// React to a hostname change: say goodbye as the old instance, then
    /// announce the new one. When the effective name is unchanged the instance
    /// is kept and only re-announced so the TXT `hostname` value stays current.
    ///
    /// On error the old instance has already been withdrawn and nothing is
    /// advertised; the handle keeps naming the most recently registered
    /// instance, so retrying the call withdraws it again (harmlessly, as
    /// `NotFound`) and registers the replacement from scratch. Callers are
    /// expected to retry.
    pub async fn rename(&mut self, hostname: impl Into<String>) -> Result<(), Error> {
        let hostname = hostname.into();
        // The new hostname is tried bare, exactly as at start: a rename away
        // from a contested name should not keep carrying its suffix.
        let effective_hostname = name::effective_hostname(&hostname, "");
        // The fast path is only valid while the recorded name is actually on
        // the air: after a failed rename nothing is advertised, and a
        // same-name retry must fall through to a full re-register.
        if effective_hostname == self.effective_hostname && self.registered {
            if hostname != self.advertisement.hostname {
                // Distinct hostnames can share one effective name (slugify
                // collapses separators); re-register the same instance to
                // refresh the TXT records — mdns-sd updates in place.
                let mut advertisement = self.advertisement.clone();
                advertisement.hostname = hostname;
                register(&self.daemon, &advertisement, &effective_hostname)?;
                self.advertisement = advertisement;
            }
            return Ok(());
        }

        // Withdraw the old instance before registering the replacement, so its
        // goodbye is emitted while it is still in a steady, announced state.
        self.registered = false;
        withdraw(&self.daemon, &self.fullname).await;

        let mut advertisement = self.advertisement.clone();
        advertisement.hostname = hostname;
        // Deliberately no attempt to put the old instance back when this
        // fails. The backend queues a command before it can report a failure,
        // so an error here does not mean the withdraw above did not happen —
        // re-registering the old name would race that queued withdraw and
        // resurrect an instance that is being retired. Leaving nothing
        // registered and letting the caller retry the whole rename converges
        // instead.
        let monitor = self.daemon.monitor()?;
        let fullname = register(&self.daemon, &advertisement, &effective_hostname)?;

        log::info!(
            "mDNS: renamed {} -> {effective_hostname}",
            self.effective_hostname
        );
        self.registered = true;
        self.fullname = fullname;
        self.advertisement = advertisement;
        self.effective_hostname = effective_hostname;
        self.probe_verdict(&monitor).await?;
        Ok(())
    }

    /// Send goodbye packets (TTL=0) for the registered instance and stop the
    /// responder, waiting until the daemon confirms so callers can order this
    /// before taking the network down.
    pub async fn shutdown(self) -> Result<(), Error> {
        let receiver = self.daemon.shutdown()?;
        timeout(SHUTDOWN_TIMEOUT, receiver.recv_async())
            .await
            .map_err(|_| Error::ShutdownTimeout)?
            .map_err(|_| Error::ShutdownConfirmationLost)?;
        log::info!("mDNS: said goodbye as {}", self.fullname);
        Ok(())
    }
}

/// Whether `original`, as reported by a backend name change, names the
/// registration described by `fullname` (service instance) or
/// `effective_hostname` (host record).
fn owns_name(fullname: &str, effective_hostname: &str, original: &str) -> bool {
    original == fullname || original.strip_suffix(".local.") == Some(effective_hostname)
}

/// Withdraw `fullname`, emitting goodbye packets (TTL=0) for it.
///
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
