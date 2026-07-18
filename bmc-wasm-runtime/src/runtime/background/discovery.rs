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

//! Background discovery helpers for the WASM runtime.

use std::time::{Duration, Instant};

use crate::host_api::{MdnsEvent, SsdpEvent, UdpBroadcastEvent};

/// Background thread for one mDNS browse session: subscribe the caller's channel
/// to the shared hub for its service types, then wait for the stop signal.
#[expect(
    clippy::needless_pass_by_value,
    reason = "thread entry point — values are moved in"
)]
pub(in crate::runtime) fn mdns_browse_thread(
    service_types: Vec<String>,
    event_tx: std::sync::mpsc::Sender<MdnsEvent>,
    stop_rx: std::sync::mpsc::Receiver<()>,
) {
    tracing::debug!("mDNS browse requested: {service_types:?}");
    let Some(hub) = mdns_hub() else {
        return;
    };
    let id = hub.subscribe(&service_types, &event_tx);
    // Block until asked to stop, or until the stop sender is dropped.
    let _ = stop_rx.recv();
    hub.unsubscribe(&service_types, id);
}

/// One shared daemon, one `browse()` per service type, fanned out to every
/// subscriber. mdns-sd 0.18.2 keys queriers by service type, so a second
/// `browse()` of a type overwrites the first's listener and a `stop_browse()`
/// stops it for everyone; the hub owns the single browse per type and refcounts
/// subscribers, deferring `stop_browse` to the last one to leave. One daemon
/// also avoids the 5353-socket contention that drops subtype PTRs.
struct MdnsHub {
    inner: std::sync::Mutex<HubInner>,
    next_id: std::sync::atomic::AtomicU64,
}

/// The daemon, its `DaemonEvent` monitor, and the per-type subscribers,
/// grouped so a recreate can swap the daemon and re-browse atomically.
struct HubInner {
    daemon: mdns_sd::ServiceDaemon,
    monitor: mdns_sd::Receiver<mdns_sd::DaemonEvent>,
    types: std::collections::HashMap<String, TypeSubscribers>,
    /// Age of the current daemon and whether it has resolved anything on it yet;
    /// together they drive the startup watchdog in [`MdnsHub::pump_once`].
    created: Instant,
    resolved_any: bool,
    /// Consecutive watchdog recreates with nothing resolved yet.
    watchdog_recreates: usize,
    /// Raw daemon events since the last (re)create, resolved or not.
    /// Zero in a degraded stretch means multicast never arrives;
    /// nonzero means packets land but resolution fails — different fixes.
    events_heard: usize,
}

struct TypeSubscribers {
    events: mdns_sd::Receiver<mdns_sd::ServiceEvent>,
    subscribers: Vec<(u64, std::sync::mpsc::Sender<MdnsEvent>)>,
    /// When this type's browse first started; survives daemon recreates
    /// so the first-resolution latency is measured from the original browse.
    started: Instant,
    first_resolved: bool,
    /// One complaint per boot once the type stays silent past the grace,
    /// so a per-type deafness shows up even while other types resolve.
    silent_warned: bool,
}

/// Grace a freshly (re)created daemon gets to resolve
/// something before the watchdog recreates it.
///
/// A restart onto an already-up network can bind sockets
/// that never receive multicast, with no `IpAdd` to trigger
/// a recreate; a fresh daemon past the old process's teardown
/// rebinds cleanly — the recovery a killall respawn gives
/// (Deck 2026-07-18: killall recovers, a bare restart does not).
///
/// Long enough that a healthy start always resolves well within it.
const DAEMON_DISCOVERY_GRACE: Duration = Duration::from_secs(8);

/// Ceiling for the watchdog's backed-off grace between recreates.
const WATCHDOG_GRACE_CAP: Duration = Duration::from_secs(300);

/// The grace before the watchdog's next recreate: [`DAEMON_DISCOVERY_GRACE`]
/// doubled per fruitless attempt, capped at [`WATCHDOG_GRACE_CAP`] —
/// so a legitimately empty network settles into one recreate per cap period
/// instead of churning and warning every few seconds forever.
fn watchdog_grace(fruitless_attempts: usize) -> Duration {
    DAEMON_DISCOVERY_GRACE
        .saturating_mul(1 << fruitless_attempts.min(6))
        .min(WATCHDOG_GRACE_CAP)
}

impl MdnsHub {
    fn subscribe(&self, service_types: &[String], tx: &std::sync::mpsc::Sender<MdnsEvent>) -> u64 {
        use std::collections::hash_map::Entry;

        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut guard = self.inner.lock().expect("BUG: mDNS hub mutex poisoned");
        let inner = &mut *guard;
        for st in service_types {
            match inner.types.entry(st.clone()) {
                Entry::Occupied(mut e) => {
                    e.get_mut().subscribers.push((id, tx.clone()));
                    tracing::debug!("mDNS: subscriber {id} joined the {st} browse");
                }
                Entry::Vacant(e) => match inner.daemon.browse(st) {
                    Ok(events) => {
                        e.insert(TypeSubscribers {
                            events,
                            subscribers: vec![(id, tx.clone())],
                            started: Instant::now(),
                            first_resolved: false,
                            silent_warned: false,
                        });
                        tracing::info!("mDNS: browse started for {st} (subscriber {id})");
                    }
                    Err(err) => tracing::error!("mDNS browse({st}) failed: {err}"),
                },
            }
        }
        id
    }

    fn unsubscribe(&self, service_types: &[String], id: u64) {
        let mut guard = self.inner.lock().expect("BUG: mDNS hub mutex poisoned");
        let inner = &mut *guard;
        for st in service_types {
            let empty = match inner.types.get_mut(st) {
                Some(subs) => {
                    subs.subscribers.retain(|(sid, _)| *sid != id);
                    subs.subscribers.is_empty()
                }
                None => continue,
            };
            if empty {
                let _ = inner.daemon.stop_browse(st);
                inner.types.remove(st);
            }
        }
    }

    /// Drain every type's daemon events and fan each out to its subscribers,
    /// dropping any whose receiver has hung up and reaping a type once
    /// its last subscriber is gone.
    ///
    /// Recreate the daemon when an interface comes up, or when a fresh one
    /// has resolved nothing within [`DAEMON_DISCOVERY_GRACE`].
    fn pump_once(&self) {
        let mut guard = self.inner.lock().expect("BUG: mDNS hub mutex poisoned");
        let inner = &mut *guard;

        // A hard power-cycle can start the daemon before the network is up,
        // binding its sockets to nothing. mdns-sd's own re-join doesn't recover it
        // here; a fresh daemon does — the recovery a restart gives. `IpAdd` fires
        // only for an interface added after startup (the initial set is silent),
        // so recreating on it never self-triggers.
        let mut interface_added = false;
        while let Ok(event) = inner.monitor.try_recv() {
            interface_added |= matches!(event, mdns_sd::DaemonEvent::IpAdd(_));
        }
        if interface_added {
            // A new interface is a changed network — retry eagerly again.
            inner.watchdog_recreates = 0;
            inner.recreate_daemon();
            tracing::info!("mDNS: recreated the daemon after an interface came up");
        }

        let mut resolved = false;
        let mut events_heard = 0;
        let mut drained = Vec::new();
        for (st, subs) in &mut inner.types {
            while let Ok(event) = subs.events.try_recv() {
                events_heard += 1;
                let Some(msg) = to_mdns_event(event) else {
                    continue;
                };
                if matches!(msg, MdnsEvent::Found(_)) {
                    resolved = true;
                    if !subs.first_resolved {
                        subs.first_resolved = true;
                        tracing::info!(
                            "mDNS: first resolution on {st} {:.1}s after its browse started",
                            subs.started.elapsed().as_secs_f32()
                        );
                    }
                }
                subs.subscribers
                    .retain(|(_, tx)| tx.send(msg.clone()).is_ok());
            }
            // A type staying silent while others resolve — the daemon watchdog
            // below can't see it. DEBUG not WARN: in the field an absent family
            // (no such miners) is silent the same way, so it isn't an alarm.
            if !subs.first_resolved
                && !subs.silent_warned
                && subs.started.elapsed() >= DAEMON_DISCOVERY_GRACE
            {
                subs.silent_warned = true;
                tracing::debug!(
                    "mDNS: no resolution on {st} within {}s of its browse — deaf to \
                     this type while others resolve; suspect fragmented responses \
                     or responder loss",
                    DAEMON_DISCOVERY_GRACE.as_secs()
                );
            }
            if subs.subscribers.is_empty() {
                drained.push(st.clone());
            }
        }
        inner.events_heard += events_heard;
        for st in drained {
            let _ = inner.daemon.stop_browse(&st);
            inner.types.remove(&st);
        }
        inner.resolved_any |= resolved;

        // Log the recovery too, so the degraded stretch has a visible end.
        if resolved && inner.watchdog_recreates > 0 {
            tracing::warn!(
                "mDNS: discovery recovered after {} watchdog recreate(s)",
                inner.watchdog_recreates
            );
            inner.watchdog_recreates = 0;
        }

        // A restart onto an already-up network draws no `IpAdd`, yet its daemon
        // can still have bound dead sockets. If nothing resolves while types
        // are subscribed, recreate once the grace passes; a killall recovers it
        // too, just coarser. WARN not INFO: a daemon that binds but never hears
        // is otherwise invisible.
        let grace = watchdog_grace(inner.watchdog_recreates);
        if !inner.resolved_any && !inner.types.is_empty() && inner.created.elapsed() >= grace {
            inner.watchdog_recreates += 1;
            let attempt = inner.watchdog_recreates;
            let heard = inner.events_heard;
            inner.recreate_daemon();
            tracing::warn!(
                "mDNS watchdog: nothing resolved in {}s ({heard} raw events heard), \
                 recreated the daemon (attempt {attempt}, next in {}s); if this \
                 repeats, discovery is degraded — check the network and the responders",
                grace.as_secs(),
                watchdog_grace(attempt).as_secs()
            );
        }
    }
}

impl HubInner {
    /// Spawn a fresh daemon with its `DaemonEvent` monitor. `None` if either fails.
    fn spawn_daemon() -> Option<(
        mdns_sd::ServiceDaemon,
        mdns_sd::Receiver<mdns_sd::DaemonEvent>,
    )> {
        let daemon = match mdns_sd::ServiceDaemon::new() {
            Ok(daemon) => daemon,
            Err(e) => {
                tracing::error!("mDNS daemon creation failed: {e}");
                return None;
            }
        };
        // Learn devices from responders' periodic/startup announcements,
        // not only from replies to our own queries — over lossy WiFi
        // multicast every extra broadcast is another chance to hear
        // a device whose reply was dropped.
        if let Err(e) = daemon.accept_unsolicited(true) {
            tracing::warn!("mDNS accept_unsolicited failed: {e}");
        }
        match daemon.monitor() {
            Ok(monitor) => Some((daemon, monitor)),
            Err(e) => {
                tracing::error!("mDNS daemon monitor failed: {e}");
                None
            }
        }
    }

    /// Replace the daemon with a fresh one and re-browse every subscribed type,
    /// so a daemon that bound before the network was up rebinds correctly.
    /// Subscriber lists are kept; only the daemon-side handles move. The caller
    /// does the logging: reason and severity differ by path.
    fn recreate_daemon(&mut self) {
        let Some((daemon, monitor)) = Self::spawn_daemon() else {
            return;
        };
        for (st, subs) in &mut self.types {
            match daemon.browse(st) {
                Ok(events) => subs.events = events,
                Err(e) => tracing::error!("mDNS re-browse({st}) failed: {e}"),
            }
        }
        let old = std::mem::replace(&mut self.daemon, daemon);
        self.monitor = monitor;
        self.created = Instant::now();
        self.resolved_any = false;
        self.events_heard = 0;
        let _ = old.shutdown();
    }
}

/// Convert an mdns-sd event into a WASM-facing [`MdnsEvent`], or `None` for the
/// lifecycle events the guest doesn't consume.
fn to_mdns_event(event: mdns_sd::ServiceEvent) -> Option<MdnsEvent> {
    use mdns_sd::ServiceEvent;

    match event {
        ServiceEvent::ServiceResolved(info) => {
            let svc_type = info.ty_domain.clone();
            let name = info.get_fullname().to_owned();
            let port = info.get_port();
            let host = info
                .get_addresses_v4()
                .iter()
                .next()
                .map(ToString::to_string)
                .unwrap_or_default();
            let txt_pairs: Vec<String> = info
                .get_properties()
                .iter()
                .map(|p| {
                    format!(
                        "\"{}\":\"{}\"",
                        escape_json(p.key()),
                        escape_json(p.val_str())
                    )
                })
                .collect();
            let txt_json = format!("{{{}}}", txt_pairs.join(","));
            let json = format!(
                "{{\"service_type\":\"{}\",\"name\":\"{}\",\"host\":\"{}\",\"port\":{},\"txt\":{}}}",
                escape_json(&svc_type),
                escape_json(&name),
                escape_json(&host),
                port,
                txt_json,
            );
            Some(MdnsEvent::Found(json))
        }
        ServiceEvent::ServiceRemoved(_, fullname) => Some(MdnsEvent::Removed(fullname)),
        ServiceEvent::SearchStarted(_)
        | ServiceEvent::ServiceFound(_, _)
        | ServiceEvent::SearchStopped(_)
        | _ => None,
    }
}

/// The process-wide mDNS hub, created on first use (`None` if the daemon can't
/// be created). Spawns the fan-out pump the first time it succeeds.
fn mdns_hub() -> Option<&'static MdnsHub> {
    static HUB: std::sync::OnceLock<Option<MdnsHub>> = std::sync::OnceLock::new();
    static PUMP: std::sync::Once = std::sync::Once::new();
    let hub = HUB
        .get_or_init(|| {
            let (daemon, monitor) = HubInner::spawn_daemon()?;
            tracing::info!("mDNS hub created");
            Some(MdnsHub {
                inner: std::sync::Mutex::new(HubInner {
                    daemon,
                    monitor,
                    types: std::collections::HashMap::new(),
                    created: Instant::now(),
                    resolved_any: false,
                    watchdog_recreates: 0,
                    events_heard: 0,
                }),
                next_id: std::sync::atomic::AtomicU64::new(0),
            })
        })
        .as_ref()?;
    PUMP.call_once(move || {
        std::thread::spawn(move || {
            loop {
                hub.pump_once();
                std::thread::sleep(Duration::from_millis(100));
            }
        });
    });
    Some(hub)
}

/// Background thread for SSDP M-SEARCH discovery.
#[expect(
    clippy::needless_pass_by_value,
    reason = "thread entry point — values are moved in"
)]
pub(in crate::runtime) fn ssdp_search_thread(
    search_target: String,
    timeout_secs: u32,
    event_tx: std::sync::mpsc::Sender<SsdpEvent>,
    stop_rx: std::sync::mpsc::Receiver<()>,
) {
    use std::collections::HashSet;
    use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

    let multicast_group = Ipv4Addr::new(239, 255, 255, 250);
    let multicast_addr = SocketAddrV4::new(multicast_group, 1900);

    let search_socket = match UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("SSDP: failed to bind search socket: {e}");
            return;
        }
    };
    if let Err(e) = search_socket.set_read_timeout(Some(Duration::from_millis(250))) {
        tracing::error!("SSDP: failed to set search socket timeout: {e}");
        return;
    }

    let notify_socket: Option<UdpSocket> =
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 1900))
            .ok()
            .and_then(|sock| {
                if let Err(e) = sock.join_multicast_v4(&multicast_group, &Ipv4Addr::UNSPECIFIED) {
                    tracing::warn!("SSDP: failed to join multicast group: {e}");
                    return None;
                }
                let _ = sock.set_read_timeout(Some(Duration::from_millis(250)));
                Some(sock)
            });

    let mut seen_usns: HashSet<String> = HashSet::new();
    let overall_timeout = Duration::from_secs(u64::from(timeout_secs).max(3));
    let resend_interval = Duration::from_secs(30);
    let mut last_send = Instant::now()
        .checked_sub(resend_interval)
        .expect("BUG: system clock too close to epoch for SSDP interval");

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        if last_send.elapsed() >= resend_interval {
            let request = format!(
                "M-SEARCH * HTTP/1.1\r\n\
                 HOST: 239.255.255.250:1900\r\n\
                 MAN: \"ssdp:discover\"\r\n\
                 MX: {timeout_secs}\r\n\
                 ST: {search_target}\r\n\r\n"
            );
            if let Err(e) = search_socket.send_to(request.as_bytes(), multicast_addr) {
                tracing::warn!("SSDP: M-SEARCH send failed: {e}");
            } else {
                tracing::debug!("SSDP: sent M-SEARCH for {search_target}");
            }
            last_send = Instant::now();
        }

        let listen_deadline = Instant::now() + overall_timeout;
        let mut buf = [0_u8; 4096];
        while Instant::now() < listen_deadline {
            if stop_rx.try_recv().is_ok() {
                return;
            }

            if let Ok((n, _addr)) = search_socket.recv_from(&mut buf) {
                let response = String::from_utf8_lossy(&buf[..n]);
                if let Some(event) = ssdp_handle_response(&response, &search_target, &mut seen_usns)
                    && event_tx.send(event).is_err()
                {
                    return;
                }
            }

            if let Some(ref sock) = notify_socket
                && let Ok((n, _addr)) = sock.recv_from(&mut buf)
            {
                let msg = String::from_utf8_lossy(&buf[..n]);
                if let Some(event) = ssdp_handle_notify(&msg, &search_target, &mut seen_usns)
                    && event_tx.send(event).is_err()
                {
                    return;
                }
            }
        }
    }
}

/// Background thread for UDP broadcast: sends a broadcast message and collects responses.
#[expect(
    clippy::needless_pass_by_value,
    reason = "thread entry point — values are moved in"
)]
pub(in crate::runtime) fn udp_broadcast_thread(
    port: u32,
    message: String,
    timeout_secs: u32,
    event_tx: std::sync::mpsc::Sender<UdpBroadcastEvent>,
    stop_rx: std::sync::mpsc::Receiver<()>,
) {
    use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

    let Ok(port) = u16::try_from(port) else {
        tracing::error!("UDP broadcast: port {port} exceeds u16 range");
        return;
    };
    let broadcast_addr = SocketAddrV4::new(Ipv4Addr::BROADCAST, port);

    let socket = match UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("UDP broadcast: failed to bind socket: {e}");
            return;
        }
    };
    if let Err(e) = socket.set_broadcast(true) {
        tracing::error!("UDP broadcast: failed to set broadcast: {e}");
        return;
    }
    if let Err(e) = socket.set_read_timeout(Some(Duration::from_millis(250))) {
        tracing::error!("UDP broadcast: failed to set read timeout: {e}");
        return;
    }

    let resend_interval = Duration::from_secs(30);
    let listen_window = Duration::from_secs(u64::from(timeout_secs).max(3));
    let mut last_send = Instant::now()
        .checked_sub(resend_interval)
        .expect("BUG: system clock too close to epoch for UDP broadcast interval");

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        if last_send.elapsed() >= resend_interval {
            if let Err(e) = socket.send_to(message.as_bytes(), broadcast_addr) {
                tracing::warn!("UDP broadcast: send failed: {e}");
            } else {
                tracing::debug!("UDP broadcast: sent to port {port}");
            }
            last_send = Instant::now();
        }

        let deadline = Instant::now() + listen_window;
        let mut buf = [0_u8; 4096];
        while Instant::now() < deadline {
            if stop_rx.try_recv().is_ok() {
                return;
            }
            if let Ok((n, addr)) = socket.recv_from(&mut buf)
                && let Ok(data) = std::str::from_utf8(&buf[..n])
            {
                let source = addr.to_string();
                if event_tx
                    .send(UdpBroadcastEvent::Response(data.to_owned(), source))
                    .is_err()
                {
                    return;
                }
            }
        }
    }
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn ssdp_handle_response(
    response: &str,
    search_target: &str,
    seen_usns: &mut std::collections::HashSet<String>,
) -> Option<SsdpEvent> {
    let st = ssdp_extract_header(response, "ST")?;
    if st != search_target {
        return None;
    }

    let location = ssdp_extract_header(response, "LOCATION")?;
    let usn = ssdp_extract_header(response, "USN")?;

    if seen_usns.contains(&usn) {
        return None;
    }
    seen_usns.insert(usn.clone());

    tracing::debug!("SSDP: discovered USN={usn} at {location}");

    if let Some(json) = ssdp_fetch_description(&location) {
        return Some(SsdpEvent::Found(json));
    }
    tracing::warn!("SSDP: failed to parse description from {location}");
    None
}

fn ssdp_handle_notify(
    msg: &str,
    search_target: &str,
    seen_usns: &mut std::collections::HashSet<String>,
) -> Option<SsdpEvent> {
    if !msg.starts_with("NOTIFY") {
        return None;
    }

    let nts = ssdp_extract_header(msg, "NTS")?;
    let usn = ssdp_extract_header(msg, "USN")?;
    let nt = ssdp_extract_header(msg, "NT").unwrap_or_default();

    if !nt.contains(search_target) && !usn.contains(search_target) {
        return None;
    }

    if nts == "ssdp:byebye" {
        tracing::debug!("SSDP: byebye USN={usn}");
        seen_usns.remove(&usn);
        Some(SsdpEvent::Removed(usn))
    } else if nts == "ssdp:alive" {
        let location = ssdp_extract_header(msg, "LOCATION")?;
        if seen_usns.contains(&usn) {
            return None;
        }
        seen_usns.insert(usn.clone());
        tracing::debug!("SSDP: alive USN={usn} at {location}");
        if let Some(json) = ssdp_fetch_description(&location) {
            return Some(SsdpEvent::Found(json));
        }
        tracing::warn!("SSDP: failed to parse description from {location}");
        None
    } else {
        None
    }
}

fn ssdp_extract_header(response: &str, header_name: &str) -> Option<String> {
    let header_lower = header_name.to_ascii_lowercase();
    for line in response.lines() {
        if let Some((key, value)) = line.split_once(':')
            && key.trim().to_ascii_lowercase() == header_lower
        {
            return Some(value.trim().to_owned());
        }
    }
    None
}

fn ssdp_fetch_description(location: &str) -> Option<String> {
    let response = ureq::get(location).call().ok()?;
    let body = response.into_body().read_to_string().ok()?;
    let doc = roxmltree::Document::parse(&body).ok()?;
    let root = doc.root_element();

    let device_elem = root.descendants().find(|n| n.has_tag_name("device"))?;
    let friendly_name = device_elem
        .descendants()
        .find(|n| n.has_tag_name("friendlyName"))
        .and_then(|n| n.text())
        .unwrap_or("Unknown");

    let mut av_transport_path = String::new();
    let mut rendering_control_path = String::new();

    for service in device_elem
        .descendants()
        .filter(|n| n.has_tag_name("service"))
    {
        let svc_type = service
            .descendants()
            .find(|n| n.has_tag_name("serviceType"))
            .and_then(|n| n.text())
            .unwrap_or("");
        let control_url = service
            .descendants()
            .find(|n| n.has_tag_name("controlURL"))
            .and_then(|n| n.text())
            .unwrap_or("");

        if svc_type.contains("AVTransport") {
            control_url.clone_into(&mut av_transport_path);
        } else if svc_type.contains("RenderingControl") {
            control_url.clone_into(&mut rendering_control_path);
        }
    }

    let url_body = location.strip_prefix("http://")?;
    let host_port = url_body.split('/').next()?;
    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
        (h, p.parse::<u16>().ok()?)
    } else {
        (host_port, 80)
    };

    let json = format!(
        "{{\"usn\":\"\",\"location\":\"{}\",\"name\":\"{}\",\"host\":\"{}\",\"port\":{},\"av_transport_path\":\"{}\",\"rendering_control_path\":\"{}\"}}",
        escape_json(location),
        escape_json(friendly_name),
        escape_json(host),
        port,
        escape_json(&av_transport_path),
        escape_json(&rendering_control_path),
    );

    Some(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watchdog_grace_doubles_to_the_cap() {
        assert_eq!(watchdog_grace(0), Duration::from_secs(8));
        assert_eq!(watchdog_grace(1), Duration::from_secs(16));
        assert_eq!(watchdog_grace(5), Duration::from_secs(256));
        assert_eq!(watchdog_grace(6), WATCHDOG_GRACE_CAP);
        assert_eq!(watchdog_grace(usize::MAX), WATCHDOG_GRACE_CAP);
    }
}
