// Copyright (C) 2026  Braiins Systems s.r.o.

//! The on-disk blueprint format and the shared resource types.
//! A [`Blueprint`] is a list of typed device [`Instance`]s; `schemars` derives
//! the exhaustive JSON schema — every device's params — from these types.

use std::collections::BTreeMap;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value as Json;

use crate::cache::Cache;
use crate::devices::{axeos, bos, ubos};
use crate::value::Value;

/// A run: the device instances to bring up.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Blueprint {
    /// Each entry instantiates one device `count` times.
    pub instances: Vec<Instance>,
}

/// One device instance, discriminated by `device`
/// and carrying that device's typed params.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "device", rename_all = "snake_case")]
pub enum Instance {
    Bos {
        /// Human label describing this entry's scenario, shown in the readout.
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        params: bos::Params,
        #[serde(default = "one")]
        count: usize,
    },
    // Braiins OS Libre is the product name; only the Rust side keeps uBOS.
    #[serde(rename = "bos-libre")]
    Ubos {
        /// Human label describing this entry's scenario, shown in the readout.
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        params: ubos::Params,
        #[serde(default = "one")]
        count: usize,
    },
    Axeos {
        /// Human label describing this entry's scenario, shown in the readout.
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        params: axeos::Params,
        #[serde(default = "one")]
        count: usize,
    },
}

fn one() -> usize {
    1
}

impl Instance {
    /// The device key, as written in the blueprint. Doubles as the announced
    /// hostname prefix, so it must stay hostname-safe.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            Instance::Bos { .. } => "bos",
            Instance::Ubos { .. } => "bos-libre",
            Instance::Axeos { .. } => "axeos",
        }
    }

    /// How many copies of this instance to bring up.
    #[must_use]
    pub fn count(&self) -> usize {
        match self {
            Instance::Bos { count, .. }
            | Instance::Ubos { count, .. }
            | Instance::Axeos { count, .. } => *count,
        }
    }

    /// The entry's human scenario label, if the blueprint set one.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        match self {
            Instance::Bos { label, .. }
            | Instance::Ubos { label, .. }
            | Instance::Axeos { label, .. } => label.as_deref(),
        }
    }

    /// Build one concrete resource from this instance's params.
    #[must_use]
    pub fn resource(&self, name: &str, port: u16) -> ResourceSpec {
        match self {
            Instance::Bos { params, .. } => params.resource(name, port),
            Instance::Ubos { params, .. } => params.resource(name, port),
            Instance::Axeos { params, .. } => params.resource(name, port),
        }
    }
}

/// A fully-resolved resource ready to advertise and serve.
#[derive(Debug, Clone)]
pub struct ResourceSpec {
    pub name: String,
    pub port: u16,
    pub announce: AnnounceSpec,
    pub endpoints: Vec<EndpointSpec>,
    /// Opt-in: the engine records these series into the device's cache for an
    /// accumulating endpoint to serve back.
    pub sampler: Option<Sampler>,
}

/// Opt-in background sampler: each tick evaluates every [`SeriesSpec`] into the
/// cache; the engine backfills to capacity at startup.
#[derive(Debug, Clone)]
pub struct Sampler {
    /// Seconds between ticks; also the spacing of the backfilled history.
    pub period_s: f64,
    pub series: Vec<SeriesSpec>,
}

/// One recorded series: a named [`Value`] sampled into a ring of `capacity`.
#[derive(Debug, Clone)]
pub struct SeriesSpec {
    pub name: String,
    pub value: Value,
    pub capacity: usize,
}

/// How a resource advertises itself. The engine assigns the port.
#[derive(Debug, Clone)]
pub enum AnnounceSpec {
    /// Advertise over mDNS/DNS-SD.
    Mdns {
        /// Base service type, e.g. `_http._tcp` or `_ubos._tcp`.
        service_type: String,
        /// Optional DNS-SD subtype without the `._sub.` glue, e.g. `_bos`.
        subtype: Option<String>,
        /// TXT record key/values.
        txt: BTreeMap<String, String>,
    },
}

impl AnnounceSpec {
    /// The DNS-SD browse string a widget uses to discover this resource,
    /// e.g. `_bos._sub._http._tcp` or `_ubos._tcp`. Readout only.
    #[must_use]
    pub fn browse(&self) -> String {
        let AnnounceSpec::Mdns {
            service_type,
            subtype,
            ..
        } = self;
        match subtype {
            Some(sub) => format!("{sub}._sub.{service_type}"),
            None => service_type.clone(),
        }
    }
}

/// A served HTTP endpoint: a template rendered per request, or a response
/// accumulated from the device's cache.
#[derive(Debug, Clone)]
pub struct EndpointSpec {
    pub method: String,
    pub path: String,
    pub body: Body,
    pub status: u16,
}

/// How an endpoint produces its response body.
#[derive(Clone)]
pub enum Body {
    /// A JSON template whose `$value` leaves are rendered per request.
    Render(Json),
    /// A response built from the device's accumulated cache (history).
    Accumulate(AccumFn),
}

/// Reads a device's cache and shapes it into an endpoint response body.
pub type AccumFn = Arc<dyn Fn(&Cache) -> Json + Send + Sync>;

impl Body {
    /// Wrap a cache reader as an accumulating endpoint body.
    #[must_use]
    pub fn accumulate<F>(reader: F) -> Self
    where
        F: Fn(&Cache) -> Json + Send + Sync + 'static,
    {
        Body::Accumulate(Arc::new(reader))
    }
}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Body::Render(template) => f.debug_tuple("Render").field(template).finish(),
            Body::Accumulate(_) => f.debug_tuple("Accumulate").finish_non_exhaustive(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Blueprint;

    /// Keep the committed `blueprint.schema.json` in lockstep with the types.
    /// Regenerate with `UPDATE_SCHEMA=1 cargo test -p bmc-netsim`.
    #[test]
    fn blueprint_schema_is_current() {
        let schema = schemars::schema_for!(Blueprint);
        let generated = format!(
            "{}\n",
            serde_json::to_string_pretty(&schema).expect("BUG: schema must serialize")
        );
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("blueprint.schema.json");
        if std::env::var_os("UPDATE_SCHEMA").is_some() {
            std::fs::write(&path, &generated).expect("BUG: writing blueprint.schema.json");
            return;
        }
        let committed = std::fs::read_to_string(&path).expect(
            "BUG: blueprint.schema.json missing; run `UPDATE_SCHEMA=1 cargo test -p bmc-netsim`",
        );
        assert_eq!(
            generated, committed,
            "blueprint.schema.json is stale — regenerate with `UPDATE_SCHEMA=1 cargo test -p bmc-netsim`",
        );
    }
}
