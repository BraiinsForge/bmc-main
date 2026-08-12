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

//! Network diagnostics shared by the support-archive collectors: interface
//! dump, public-IP discovery, ping report, and per-interface packet capture.

use anyhow::{Context, Result, anyhow};
use bmc_net_dns::IiResolver;
use pcap_file::DataLink;
use pcap_file::pcapng::PcapNgWriter;
use pcap_file::pcapng::blocks::enhanced_packet::EnhancedPacketBlock;
use pcap_file::pcapng::blocks::interface_description::InterfaceDescriptionBlock;
use pnet::datalink::{self, Channel, Config, NetworkInterface};
use reqwest::blocking::Client;
use std::borrow::Cow;
use std::collections::HashMap;
use std::convert::TryInto;
use std::io::{Cursor, ErrorKind};
use std::net::{IpAddr, SocketAddr};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tracing::{debug, error, info, warn};

/// Renders a human-readable dump of all local network interfaces.
#[must_use]
pub fn ifconfig() -> String {
    datalink::interfaces()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Discovers the device's public IP by querying well-known reflectors,
/// returning the first response that parses as a valid IP address.
///
/// Responses are validated (`str::parse::<IpAddr>`) rather than trusted
/// verbatim, so a broken or hostile provider cannot inject arbitrary text into
/// the support archive. Plain HTTP is used deliberately: this crate builds
/// reqwest without a TLS backend, so HTTPS is not available here.
pub fn public_ip() -> Result<String> {
    const PUBLIC_IP_PROVIDERS: &[&str] = &[
        "http://ipinfo.io/ip",
        "http://ipecho.net/plain",
        "http://checkip.amazonaws.com",
    ];
    const TIMEOUT: Duration = Duration::from_secs(2);

    let client = Client::builder()
        .timeout(TIMEOUT)
        .build()
        .expect("BUG: incorrect builder");

    PUBLIC_IP_PROVIDERS
        .iter()
        .find_map(|&url| {
            let body = client.get(url).send().ok()?.text().ok()?;
            body.trim().parse::<IpAddr>().ok()
        })
        .map(|ip| format!("{ip}\n"))
        .context("not a single public IP provider returned a valid address")
}

/// Resolves and pings each host over both IPv4 and IPv6, returning a formatted
/// reachability table plus a legend of the distinct outcomes observed.
pub fn ping_report(hosts: &[&str]) -> Result<String> {
    let hosts_col_width = hosts.iter().map(|x| x.len()).max().unwrap_or_default();
    let mut labels = HashMap::new();
    let mut err_counter = 0;

    let lines: Vec<_> = hosts
        .iter()
        .map(|&hostname| {
            // look up both the IPv4 and IPv6 address for the given hostname
            let (ipv4, ipv6) = match IiResolver::lookup_host_sync((hostname, 0)) {
                Ok(lookup) => {
                    let lookup: Vec<SocketAddr> = lookup.collect();
                    let ipv4 = lookup.iter().find_map(|a| a.is_ipv4().then(|| a.ip()));
                    let ipv6 = lookup.iter().find_map(|a| a.is_ipv6().then(|| a.ip()));
                    (ipv4, ipv6)
                }
                Err(_) => (None, None),
            };

            let [status4, status6] = [ipv4, ipv6].map(|addr| {
                if let Some(addr) = addr {
                    const PING_TIMEOUT: Duration = Duration::from_secs(2);
                    match ping::ping(addr, Some(PING_TIMEOUT), None, None, None, None) {
                        Ok(()) => {
                            let label = "OK";
                            labels.insert(
                                "the ICMP reply returned successfully".to_owned(),
                                label.to_owned(),
                            );
                            label.to_owned()
                        }
                        Err(err) => {
                            let message = err.to_string();
                            labels
                                .entry(message)
                                .or_insert_with(|| {
                                    err_counter += 1;
                                    format!("ERR{err_counter}")
                                })
                                .clone()
                        }
                    }
                } else {
                    let label = "N/A";
                    labels.insert("failed to resolve hostname".to_owned(), label.to_owned());
                    label.to_owned()
                }
            });

            format!(
                "{: <hcw$} {: <7} {: <7} {: <16} {}",
                hostname,
                status4,
                status6,
                ipv4.map_or_else(|| "-".to_owned(), |x| x.to_string()),
                ipv6.map_or_else(|| "-".to_owned(), |x| x.to_string()),
                hcw = hosts_col_width + 2
            )
        })
        .collect();

    if lines.is_empty() {
        return Err(anyhow!("at least one host must be specified"));
    }

    let header = format!(
        "{: <hcw$} {: <7} {: <7} {: <16} {}",
        "Hostname",
        "Ping4",
        "Ping6",
        "Resolved IPv4",
        "Resolved IPv6",
        hcw = hosts_col_width + 2
    );

    let description = {
        let mut desc: Vec<_> = labels
            .into_iter()
            .map(|(message, label)| format!("{:.<9} {}", format!("{} ", label), message))
            .collect();
        desc.sort();
        desc
    };

    let report = {
        let mut r = vec![];
        r.push("NOTE: This uses public DNS resolvers provided by Google.");
        r.push("");
        r.push(&header);
        r.extend(lines.iter().map(String::as_str));
        r.push("");
        r.extend(description.iter().map(String::as_str));
        r.push("");
        r.join("\n")
    };

    Ok(report)
}

/// Captures traffic on `interface` for up to `duration` and returns a pcapng
/// buffer. Capture also stops early once a fixed byte cap (16 MiB) is reached,
/// so a busy interface cannot exhaust memory on a constrained BMC.
pub fn pcap(interface: &NetworkInterface, duration: Duration) -> Result<Vec<u8>, PcapError> {
    const READ_TIMEOUT: Duration = Duration::from_millis(100);
    // Cap in-memory capture size (16 MiB) so a busy link can't exhaust RAM.
    const MAX_CAPTURE_BYTES: usize = 16 * 1024 * 1024;

    info!("Starting packet capture on interface: {}", interface.name);

    let mut buffer = vec![];
    let mut pcap_ng_writer =
        PcapNgWriter::new(Cursor::new(&mut buffer)).expect("BUG: pcap writing failed");

    let idb = InterfaceDescriptionBlock {
        linktype: DataLink::ETHERNET,
        snaplen: 0xFFFF,
        options: vec![],
    };

    pcap_ng_writer.write_pcapng_block(idb)?;

    let config = Config {
        // Since `rx.next()` is a blocking call, we want it to return once in a while when there are no incoming
        // ethernet frames so that we can check if the specified capture duration has elapsed.
        read_timeout: Some(READ_TIMEOUT),
        read_buffer_size: u16::MAX.into(),
        ..Default::default()
    };

    let Channel::Ethernet(_tx, mut rx) = datalink::channel(interface, config)? else {
        return Err(PcapError::UnsupportedInterface(interface.name.clone()));
    };

    let start_time = Instant::now();
    // Tracked separately because `buffer` is mutably borrowed by the writer.
    let mut captured_bytes = 0_usize;
    while start_time.elapsed() < duration {
        if captured_bytes >= MAX_CAPTURE_BYTES {
            warn!(
                "Packet capture on '{}' hit the {MAX_CAPTURE_BYTES}-byte cap; stopping early",
                interface.name
            );
            break;
        }
        match rx.next() {
            Ok(frame) => {
                debug!("Received packet on interface '{}'", interface.name);

                captured_bytes += frame.len();
                let mut epb = EnhancedPacketBlock::default();

                epb.interface_id = 0;
                epb.timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_else(|_| {
                        warn!("System time is before UNIX epoch, using fallback timestamp");
                        Duration::ZERO // Fallback to UNIX epoch
                    });
                epb.original_len = frame
                    .len()
                    .try_into()
                    .map_err(|_| PcapError::OverflowError)?;
                epb.data = Cow::Borrowed(frame);

                pcap_ng_writer.write_pcapng_block(epb)?;
            }
            Err(io_error) if io_error.kind() == ErrorKind::TimedOut => continue,
            Err(io_error) => {
                let msg = format!("I/O error on {}: {}", interface.name, io_error);
                error!("{}", msg);
                return Err(PcapError::IoError(std::io::Error::other(msg)));
            }
        }
    }
    Ok(buffer)
}

/// One interface's capture result: the interface name paired with the pcapng
/// bytes or the [`PcapError`]. The name is captured before the capture starts,
/// so a failed capture still says which interface it belongs to.
pub type PcapResult = (String, Result<Vec<u8>, PcapError>);

/// A set of per-interface packet captures running concurrently, returned by
/// [`pcap_all`]. Enumeration and threading are internal so callers do not need
/// to depend on `pnet` or manage threads themselves.
#[must_use]
#[derive(Debug)]
pub struct PcapCapture {
    handles: Vec<JoinHandle<PcapResult>>,
}

/// Starts a [`pcap`] capture on every local interface concurrently for
/// `duration` and returns a handle to collect the results with
/// [`PcapCapture::collect`]. The captures run in the background so the caller
/// can do other work while they proceed.
pub fn pcap_all(duration: Duration) -> PcapCapture {
    let handles = datalink::interfaces()
        .into_iter()
        .map(|interface| {
            thread::spawn(move || {
                // Bind the name up front so the pairing survives a failure.
                let name = interface.name.clone();
                (name, pcap(&interface, duration))
            })
        })
        .collect();
    PcapCapture { handles }
}

impl PcapCapture {
    /// Waits for every capture to finish and returns each interface's name with
    /// its result (the pcapng bytes on success, or the [`PcapError`]). Threads
    /// that panic are logged and dropped from the result.
    #[must_use]
    pub fn collect(self) -> Vec<PcapResult> {
        self.handles
            .into_iter()
            .filter_map(|handle| {
                handle.join().ok().or_else(|| {
                    error!("Thread panicked while capturing pcap data");
                    None
                })
            })
            .collect()
    }
}

/// Errors that can occur while capturing packets in [`pcap`].
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum PcapError {
    #[error("Unsupported channel type for interface: {0}")]
    UnsupportedInterface(String),
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Pcap error: {0}")]
    Pcap(#[from] pcap_file::PcapError),
    #[error("Overflow error")]
    OverflowError,
}
