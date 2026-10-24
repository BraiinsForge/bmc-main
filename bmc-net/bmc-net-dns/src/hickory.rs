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

use std::{fmt::Debug, io::Error, net::SocketAddr, time::Duration};

use hickory_resolver::{
    Resolver, TokioResolver,
    config::{
        GOOGLE, LookupIpStrategy, ResolveHosts, ResolverConfig, ResolverOpts,
        ServerOrderingStrategy,
    },
    net::{NetError, runtime::TokioRuntimeProvider},
    system_conf::read_system_conf,
};
use log::error;
use rand::rng;
use rand::seq::SliceRandom;

use crate::ToHostAndPortTuple;

/// Holds the hickory system resolver (built from `/etc/resolv.conf`) and builds
/// the Google-DNS fallback resolver on demand.
pub struct HickoryResolverBuilder {
    system_resolver: Option<TokioResolver>,
}

impl Default for HickoryResolverBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl HickoryResolverBuilder {
    pub fn new() -> Self {
        Self {
            system_resolver: Self::build_system_resolver(),
        }
    }

    fn get_resolver_options() -> ResolverOpts {
        let mut opts = ResolverOpts::default();
        opts.use_hosts_file = ResolveHosts::Always;
        opts.edns0 = true;
        opts.server_ordering_strategy = ServerOrderingStrategy::UserProvidedOrder;
        opts.ip_strategy = LookupIpStrategy::Ipv4AndIpv6;
        opts.negative_max_ttl = Some(Duration::ZERO);
        opts.attempts = 1;
        opts.cache_size = 0;

        opts
    }

    fn build_system_resolver() -> Option<TokioResolver> {
        match read_system_conf().and_then(|(config, _)| {
            Resolver::builder_with_config(
                Self::distrust_negative_responses(config),
                TokioRuntimeProvider::default(),
            )
            .with_options(Self::get_resolver_options())
            .build()
        }) {
            Ok(resolver) => Some(resolver),
            Err(e) => {
                error!("Cannot initialize system DNS resolver: {e}");
                None
            }
        }
    }

    // hickory 0.26 trusts an NXDOMAIN from a resolv.conf server and asks no other;
    // 0.25 did not trust it and went on to the next server.
    fn distrust_negative_responses(mut config: ResolverConfig) -> ResolverConfig {
        for server in &mut config.name_servers {
            server.trust_negative_responses = false;
        }
        config
    }

    // Since hickory 0.26 `ResolverConfig::default()` lists no name servers.
    fn google_config() -> ResolverConfig {
        ResolverConfig::udp_and_tcp(&GOOGLE)
    }

    fn build_google_resolver() -> Result<TokioResolver, NetError> {
        Resolver::builder_with_config(Self::google_config(), TokioRuntimeProvider::default())
            .with_options(Self::get_resolver_options())
            .build()
    }

    pub async fn lookup_host_google<T>(
        &self,
        host: T,
    ) -> Result<impl Iterator<Item = SocketAddr> + Send + use<T>, Error>
    where
        T: ToHostAndPortTuple + Debug + Clone + Send + Sync,
    {
        let resolver = Self::build_google_resolver().map_err(Error::other)?;
        self.lookup_host_internal(resolver, host).await
    }

    pub async fn lookup_host_system<T>(
        &self,
        host: T,
    ) -> Result<impl Iterator<Item = SocketAddr> + Send + use<T>, Error>
    where
        T: ToHostAndPortTuple + Debug + Clone + Send + Sync,
    {
        if let Some(resolver) = self.system_resolver.clone() {
            Ok(self.lookup_host_internal(resolver, host).await?)
        } else {
            Err(Error::other("Resolver not initialized"))
        }
    }

    async fn lookup_host_internal<T>(
        &self,
        resolver: TokioResolver,
        host: T,
    ) -> Result<impl Iterator<Item = SocketAddr> + Send + use<T>, Error>
    where
        T: ToHostAndPortTuple + Debug + Clone + Send + Sync,
    {
        let (host, port) = host.to_host_and_port_tuple();

        let mut result: Vec<SocketAddr> = resolver
            .lookup_ip(host)
            .await
            .map_err(Error::other)?
            .into_iter()
            .map(move |addr| SocketAddr::new(addr, port))
            .collect();
        result.shuffle(&mut rng());

        Ok(result.into_iter())
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use hickory_resolver::system_conf::parse_resolv_conf;

    use super::HickoryResolverBuilder;

    #[test]
    fn google_fallback_queries_google_dns() {
        let servers: Vec<IpAddr> = HickoryResolverBuilder::google_config()
            .name_servers
            .iter()
            .map(|server| server.ip)
            .collect();

        assert!(
            servers.contains(&IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))),
            "the Google fallback must list Google DNS; an empty server list \
             resolves nothing, got {servers:?}"
        );
    }

    #[test]
    fn system_servers_pass_an_nxdomain_on_to_the_next_server() {
        let (config, _) = parse_resolv_conf("nameserver 127.0.0.1\nnameserver ::1\n")
            .expect("BUG: a two-server resolv.conf parses");
        let config = HickoryResolverBuilder::distrust_negative_responses(config);

        assert_eq!(
            config.name_servers.len(),
            2,
            "both resolv.conf servers are kept"
        );
        assert!(
            config
                .name_servers
                .iter()
                .all(|server| !server.trust_negative_responses),
            "an NXDOMAIN from one resolv.conf server must let the next one answer, \
             as hickory 0.25 did"
        );
    }
}
