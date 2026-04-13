// Copyright (C) 2026  Braiins Systems s.r.o.

//! Background discovery helpers for the WASM runtime.

use std::time::{Duration, Instant};

use crate::host_api::{MdnsEvent, SsdpEvent, UdpBroadcastEvent};

/// Background thread for mDNS browse sessions.
#[expect(
    clippy::needless_pass_by_value,
    reason = "thread entry point — values are moved in"
)]
pub(in crate::runtime) fn mdns_browse_thread(
    service_types: Vec<String>,
    event_tx: std::sync::mpsc::Sender<MdnsEvent>,
    stop_rx: std::sync::mpsc::Receiver<()>,
) {
    use mdns_sd::{ServiceDaemon, ServiceEvent};

    let daemon = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("mDNS daemon creation failed: {e}");
            return;
        }
    };

    let receivers: Vec<_> = service_types
        .iter()
        .filter_map(|st| match daemon.browse(st) {
            Ok(rx) => Some((st.clone(), rx)),
            Err(e) => {
                tracing::error!("mDNS browse({st}) failed: {e}");
                None
            }
        })
        .collect();

    if receivers.is_empty() {
        let _ = daemon.shutdown();
        return;
    }

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }
        for (_, rx) in &receivers {
            while let Ok(event) = rx.try_recv() {
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
                                let k = p.key();
                                let v = p.val_str();
                                format!("\"{}\":\"{}\"", escape_json(k), escape_json(v))
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
                        if event_tx.send(MdnsEvent::Found(json)).is_err() {
                            break;
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        if event_tx.send(MdnsEvent::Removed(fullname)).is_err() {
                            break;
                        }
                    }
                    ServiceEvent::SearchStarted(_)
                    | ServiceEvent::ServiceFound(_, _)
                    | ServiceEvent::SearchStopped(_)
                    | _ => {}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = daemon.shutdown();
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
