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

/// The base TCP port; the `n`-th resource listens on `BASE_PORT + n`.
const BASE_PORT: u16 = 20_000;

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
    let mut per_device: HashMap<&str, usize> = HashMap::new();
    let mut index: u16 = 0;
    for instance in &blueprint.instances {
        let key = instance.key();
        let label = instance.label().unwrap_or("(unlabeled)");
        for _ in 0..instance.count() {
            let seq = {
                let entry = per_device.entry(key).or_insert(0);
                *entry += 1;
                *entry
            };
            let port = BASE_PORT
                .checked_add(index)
                .ok_or_else(|| anyhow!("too many resources; port range exhausted"))?;
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
