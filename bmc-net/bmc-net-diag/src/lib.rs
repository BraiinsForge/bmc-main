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
//! dump, public-IP discovery, and ping report.

use anyhow::{Context, Result, anyhow};
use bmc_net_dns::IiResolver;
use pnet::datalink;
use reqwest::blocking::Client;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

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
