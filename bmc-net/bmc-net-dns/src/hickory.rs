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
    Resolver,
    config::{
        LookupIpStrategy, ResolveHosts, ResolverConfig, ResolverOpts, ServerOrderingStrategy,
    },
    name_server::{GenericConnector, TokioConnectionProvider},
    proto::runtime::TokioRuntimeProvider,
};
use log::error;
use rand::rng;
use rand::seq::SliceRandom;

use crate::ToHostAndPortTuple;

/// Holds the hickory system resolver (built from `/etc/resolv.conf`) and builds
/// the Google-DNS fallback resolver on demand.
pub struct HickoryResolverBuilder {
    system_resolver: Option<Resolver<GenericConnector<TokioRuntimeProvider>>>,
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

    fn build_system_resolver() -> Option<Resolver<GenericConnector<TokioRuntimeProvider>>> {
        match Resolver::builder_tokio() {
            Ok(resolver) => Some(resolver.with_options(Self::get_resolver_options()).build()),
            Err(e) => {
                error!("Cannot initialize system DNS resolver: {}", e);
                None
            }
        }
    }

    fn build_google_resolver() -> Resolver<GenericConnector<TokioRuntimeProvider>> {
        Resolver::builder_with_config(
            ResolverConfig::default(),
            TokioConnectionProvider::default(),
        )
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
        self.lookup_host_internal(Self::build_google_resolver(), host)
            .await
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
        resolver: Resolver<GenericConnector<TokioRuntimeProvider>>,
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
