// Copyright (C) 2025  Braiins Systems s.r.o.
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

//! DNS resolution for the `bmc-net` crate set.
//!
//! [`IiResolver`] resolves host names through a fallback ladder — the tokio
//! (OS `getaddrinfo`) resolver first, then hickory's system resolver, then an
//! optional Google-DNS fallback gated on a flag file — and [`IiTcpStream`]
//! layers TCP connection on top. Both accept anything implementing
//! [`ToHostAndPortTuple`], including bracketed IPv6 literals (`[::1]`).

use std::{fmt::Debug, io::Error, net::SocketAddr, path::Path, time::Duration};

use log::{error, info, warn};
use tokio::{
    net::TcpStream,
    time::{MissedTickBehavior, interval},
};

use crate::hickory::HickoryResolverBuilder;

mod hickory;

/// Conversion into the `(host, port)` pair the resolvers accept.
pub trait ToHostAndPortTuple {
    /// Returns the host (with any surrounding IPv6 brackets stripped) and port.
    fn to_host_and_port_tuple(&self) -> (String, u16);
}

impl<T: ToString> ToHostAndPortTuple for (T, u16) {
    fn to_host_and_port_tuple(&self) -> (String, u16) {
        let host = self.0.to_string();
        // Normalize bracketed IPv6 literals (e.g. "[::1]" -> "::1") so both the
        // tuple-based tokio path and hickory's `lookup_ip` accept the host.
        let host = match host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            Some(unbracketed) => unbracketed.to_owned(),
            None => host,
        };
        (host, self.1)
    }
}

#[derive(Debug)]
pub struct IiResolver;

impl IiResolver {
    const RETRY_DELAY: Duration = Duration::from_secs(1);
    const TOKIO_RETRIES: u8 = 2;
    const GOOGLE_DNS_FLAG_FILE_PATH: &str = "/etc/google-dns-fallback";

    pub async fn lookup_host<T>(host: T) -> Result<impl Iterator<Item = SocketAddr> + Send, Error>
    where
        T: ToHostAndPortTuple + Debug + Clone + Send + Sync,
    {
        let mut interval = interval(Self::RETRY_DELAY);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        for attempt in 1..=Self::TOKIO_RETRIES {
            interval.tick().await;
            match tokio::net::lookup_host(host.clone().to_host_and_port_tuple()).await {
                Ok(iterator) => {
                    info!("Host {host:?} resolved by tokio resolver successfully");
                    return Ok(iterator.collect::<Vec<SocketAddr>>().into_iter());
                }
                Err(e) => warn!(
                    "Unable to resolve {:?} using tokio resolver (try: {}/{}): {}",
                    host,
                    attempt,
                    Self::TOKIO_RETRIES,
                    e
                ),
            }
        }

        let hickory_resolver = HickoryResolverBuilder::new();
        match hickory_resolver
            .lookup_host_system(host.clone().to_host_and_port_tuple())
            .await
        {
            Ok(iterator) => {
                info!("Host {host:?} resolved by hickory system resolver successfully");
                return Ok(iterator.collect::<Vec<SocketAddr>>().into_iter());
            }
            Err(e) => warn!("Unable to resolve {host:?} using hickory system resolver: {e}"),
        }

        if Path::new(Self::GOOGLE_DNS_FLAG_FILE_PATH).exists() {
            match hickory_resolver
                .lookup_host_google(host.clone().to_host_and_port_tuple())
                .await
            {
                Ok(iterator) => {
                    info!("Host {host:?} resolved by hickory google resolver successfully");
                    return Ok(iterator.collect::<Vec<SocketAddr>>().into_iter());
                }
                Err(e) => {
                    warn!("Unable to resolve {host:?} using hickory google resolver: {e}");
                }
            }
        }

        let err_msg = format!("Failed to resolve {host:?}");
        error!("{err_msg}");
        Err(Error::other(err_msg))
    }

    /// Blocking wrapper around [`lookup_host`](Self::lookup_host) for use from
    /// synchronous code (e.g. the support-archive collector).
    ///
    /// It builds a dedicated current-thread Tokio runtime so the async lookup
    /// has the reactor and timer it needs. Do **not** call this from inside an
    /// async task — use [`lookup_host`](Self::lookup_host) there instead.
    pub fn lookup_host_sync<T>(host: T) -> Result<impl Iterator<Item = SocketAddr> + Send, Error>
    where
        T: ToHostAndPortTuple + Debug + Clone + Send + Sync,
    {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(Self::lookup_host(host))
    }
}
#[derive(Debug)]
pub struct IiTcpStream;

impl IiTcpStream {
    pub async fn connect<T>(host: T) -> Result<TcpStream, Error>
    where
        T: ToHostAndPortTuple + Debug + Clone + Send + Sync,
    {
        let host_desc = format!("{host:?}");
        let addrs: Vec<SocketAddr> = IiResolver::lookup_host(host).await?.collect();
        let mut last_err = None;
        for addr in addrs {
            match TcpStream::connect(addr).await {
                Ok(tcpstream) => {
                    info!("TcpStream connected to {addr}");
                    return Ok(tcpstream);
                }
                Err(e) => last_err = Some(e),
            }
        }

        Err(last_err
            .unwrap_or_else(|| Error::other(format!("no addresses resolved for {host_desc}"))))
    }
}
