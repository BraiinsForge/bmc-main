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

//! `bmc-netsim` — a generic mDNS + REST network-resource simulator.
//! A [`Blueprint`] lists typed device instances; [`serve`] brings each up
//! on the real LAN so widgets discover and poll them as if they were real hardware.
//! The engine is device-agnostic — device knowledge lives entirely
//! in [`devices`](crate::devices) profile modules.

pub mod announce;
pub mod blueprint;
pub mod build;
pub mod cache;
pub mod devices;
pub mod diag;
pub mod http_status;
pub mod noise;
pub mod quantity;
pub mod render;
pub mod respond;
pub mod sampler;
pub mod value;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Result, anyhow};

use crate::announce::{Announcer, MdnsAnnouncer};
use crate::blueprint::{AnnounceSpec, Blueprint, ResourceSpec};
use crate::cache::Cache;

/// The base TCP port; unpinned resources fill upward from here.
const BASE_PORT: u16 = 20_000;

/// Every instance copy's port, in bring-up order: a pinned entry claims
/// `port..port+count`, the rest fill from [`BASE_PORT`] upward while
/// skipping the claimed ports, so pinned and auto entries cannot collide.
fn port_plan(blueprint: &Blueprint) -> Result<Vec<u16>> {
    let mut claimed = std::collections::HashSet::new();
    for instance in &blueprint.instances {
        if let Some(base) = instance.port() {
            for copy in 0..instance.count() {
                let port = offset_port(base, copy)?;
                if !claimed.insert(port) {
                    return Err(anyhow!(
                        "port {port} pinned more than once in the blueprint"
                    ));
                }
            }
        }
    }
    let mut next_auto = u32::from(BASE_PORT);
    let mut plan = Vec::new();
    for instance in &blueprint.instances {
        for copy in 0..instance.count() {
            let port = if let Some(base) = instance.port() {
                offset_port(base, copy)?
            } else {
                let mut candidate = next_auto;
                while u16::try_from(candidate).is_ok_and(|port| claimed.contains(&port)) {
                    candidate += 1;
                }
                let port = u16::try_from(candidate)
                    .map_err(|_| anyhow!("too many resources; port range exhausted"))?;
                next_auto = candidate + 1;
                port
            };
            plan.push(port);
        }
    }
    Ok(plan)
}

fn offset_port(base: u16, copy: usize) -> Result<u16> {
    u32::try_from(copy)
        .ok()
        .map(|copy| u32::from(base) + copy)
        .and_then(|port| u16::try_from(port).ok())
        .ok_or_else(|| anyhow!("too many resources; port range exhausted"))
}

/// Base for the per-instance seed: reproducible run-to-run, distinct per device.
const SEED_BASE: u64 = 0x6E65_7473_696D_0001;

/// Bring up every instance in `blueprint` — advertise over mDNS
/// and serve its HTTP endpoints — then run until Ctrl-C.
///
/// # Errors
/// Fails if the mDNS daemon cannot start, a resource's port is unavailable,
/// or the fleet exceeds the port range.
pub async fn serve(blueprint: Blueprint) -> Result<()> {
    let announcer = MdnsAnnouncer::new()?;
    let plan = port_plan(&blueprint)?;
    let mut per_device: HashMap<&str, usize> = HashMap::new();
    let mut index: usize = 0;
    for instance in &blueprint.instances {
        let key = instance.key();
        let label = instance.label().unwrap_or("(unlabeled)");
        for _ in 0..instance.count() {
            let seq = {
                let entry = per_device.entry(key).or_insert(0);
                *entry += 1;
                *entry
            };
            let port = plan[index];
            let name = format!("{key}-{seq:02}");
            let ResourceSpec {
                name,
                port,
                announce,
                endpoints,
                sampler,
            } = instance.resource(&name, port);
            let endpoint_count = endpoints.len();
            let seed = noise::mix(SEED_BASE, &name);
            let start = Instant::now();
            let series = sampler
                .as_ref()
                .map(|s| {
                    s.series
                        .iter()
                        .map(|x| (x.name.clone(), x.capacity))
                        .collect()
                })
                .unwrap_or_default();
            let cache = Arc::new(Cache::new::<Vec<_>>(series));
            // Hold the port before advertising, so a device is never announced
            // with no API behind it.
            let bound = respond::bind(port, endpoints, seed, start, Arc::clone(&cache)).await?;
            if let Some(announce) = &announce {
                announcer.announce(&name, port, announce)?;
            }
            tracing::info!(
                name = %name,
                mdns = %announce
                    .as_ref()
                    .map_or_else(|| "(cloud, not announced)".to_owned(), AnnounceSpec::browse),
                port,
                endpoints = endpoint_count,
                "up: {label}",
            );
            if let Some(sampler) = sampler {
                tokio::spawn(sampler::run(Arc::clone(&cache), sampler, seed, start));
            }
            tokio::spawn(async move {
                if let Err(err) = bound.serve().await {
                    tracing::error!(name = %name, "responder stopped: {err:#}");
                }
            });
            index += 1;
        }
    }
    tracing::info!(devices = index, "fleet up — Ctrl-C to stop");
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");
    Ok(())
}

#[cfg(test)]
mod port_plan_tests {
    use super::*;
    use crate::blueprint::Instance;
    use crate::devices::{bos, braiins_pool};

    fn pool(port: Option<u16>, count: usize) -> Instance {
        Instance::BraiinsPool {
            label: None,
            params: braiins_pool::Params::default(),
            count,
            port,
        }
    }

    fn bos_auto(count: usize) -> Instance {
        Instance::Bos {
            label: None,
            params: bos::Params::default(),
            count,
            port: None,
        }
    }

    #[test]
    fn pinned_entries_claim_their_range_and_autos_skip_it() {
        let blueprint = Blueprint {
            instances: vec![bos_auto(2), pool(Some(20_001), 2)],
        };
        assert_eq!(
            port_plan(&blueprint).expect("mixed plan resolves"),
            vec![20_000, 20_003, 20_001, 20_002],
        );
    }

    #[test]
    fn a_port_pinned_twice_is_refused() {
        let blueprint = Blueprint {
            instances: vec![pool(Some(20_005), 1), pool(Some(20_004), 2)],
        };
        let err = port_plan(&blueprint).expect_err("overlapping pins are refused");
        assert!(format!("{err:#}").contains("20005"), "{err:#}");
    }
}
