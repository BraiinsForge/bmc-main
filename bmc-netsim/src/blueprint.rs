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

//! The on-disk blueprint format and the shared resource types.
//! A [`Blueprint`] is a list of typed device [`Instance`]s; `schemars` derives
//! the exhaustive JSON schema — every device's params — from these types.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use std::fmt;

use schemars::JsonSchema;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value as Json;

use crate::cache::Cache;
use crate::devices::{axeos, bos, braiins_pool, formula_1, ubos};
use crate::http_status::HttpStatus;
use crate::value::Value;

/// A run: the device instances to bring up.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Blueprint {
    /// Each entry instantiates one device `count` times.
    pub instances: Vec<Instance>,
}

impl Blueprint {
    /// Anchor every path an instance names to `base`,
    /// the directory the blueprint was read from.
    pub fn resolve_paths(&mut self, base: &Path) {
        for instance in &mut self.instances {
            if let Instance::Formula1 { params, .. } = instance {
                params.resolve_paths(base);
            }
        }
    }
}

/// One device instance, discriminated by `device`
/// and carrying that device's typed params.
// The `serde` attributes are here for `schemars`, which reads them to shape the
// schema; `Deserialize` is hand-written below rather than derived.
#[derive(Debug, Clone, JsonSchema)]
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
        /// Pinned TCP port for this entry (a `count` fans out from it);
        /// omitted = auto-assigned from the base port upward.
        #[serde(default)]
        port: Option<u16>,
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
        /// Pinned TCP port for this entry (a `count` fans out from it);
        /// omitted = auto-assigned from the base port upward.
        #[serde(default)]
        port: Option<u16>,
    },
    Axeos {
        /// Human label describing this entry's scenario, shown in the readout.
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        params: axeos::Params,
        #[serde(default = "one")]
        count: usize,
        /// Pinned TCP port for this entry (a `count` fans out from it);
        /// omitted = auto-assigned from the base port upward.
        #[serde(default)]
        port: Option<u16>,
    },
    /// A Braiins Pool account — a cloud API on its port, never announced.
    #[serde(rename = "braiins-pool")]
    BraiinsPool {
        /// Human label describing this entry's scenario, shown in the readout.
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        params: braiins_pool::Params,
        #[serde(default = "one")]
        count: usize,
        /// Pinned TCP port for this entry (a `count` fans out from it);
        /// omitted = auto-assigned from the base port upward.
        #[serde(default)]
        port: Option<u16>,
    },
    /// A Nexus Formula 1 deployment — a cloud API on its port, never announced.
    #[serde(rename = "formula-1")]
    Formula1 {
        /// Human label describing this entry's scenario, shown in the readout.
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        params: formula_1::Params,
        #[serde(default = "one")]
        count: usize,
        /// Pinned TCP port for this entry (a `count` fans out from it);
        /// omitted = auto-assigned from the base port upward.
        #[serde(default)]
        port: Option<u16>,
    },
}

fn one() -> usize {
    1
}

/// The device keys a blueprint may name, as written in the `device` field.
const DEVICE_KEYS: &[&str] = &["bos", "bos-libre", "axeos", "braiins-pool", "formula-1"];
const INSTANCE_FIELDS: &[&str] = &["device", "label", "params", "count", "port"];

/// One device's typed params, before they are folded into an [`Instance`].
enum DeviceParams {
    Bos(bos::Params),
    Ubos(ubos::Params),
    Axeos(axeos::Params),
    BraiinsPool(braiins_pool::Params),
    Formula1(formula_1::Params),
}

impl DeviceParams {
    /// Read the params straight from the map, so the underlying format keeps
    /// its source span for anything the params reject.
    fn read<'de, A: MapAccess<'de>>(device: &str, map: &mut A) -> Result<Self, A::Error> {
        Ok(match device {
            "bos" => DeviceParams::Bos(map.next_value()?),
            "bos-libre" => DeviceParams::Ubos(map.next_value()?),
            "axeos" => DeviceParams::Axeos(map.next_value()?),
            "braiins-pool" => DeviceParams::BraiinsPool(map.next_value()?),
            "formula-1" => DeviceParams::Formula1(map.next_value()?),
            other => return Err(de::Error::unknown_variant(other, DEVICE_KEYS)),
        })
    }

    /// Re-read params that arrived before the `device` naming their type. The
    /// span is already lost by the time they can be typed, so this path only
    /// keeps such blueprints working — put `device` first to keep the caret.
    fn reparse<E: de::Error>(device: &str, json: Json) -> Result<Self, E> {
        Ok(match device {
            "bos" => DeviceParams::Bos(serde_json::from_value(json).map_err(E::custom)?),
            "bos-libre" => DeviceParams::Ubos(serde_json::from_value(json).map_err(E::custom)?),
            "axeos" => DeviceParams::Axeos(serde_json::from_value(json).map_err(E::custom)?),
            "braiins-pool" => {
                DeviceParams::BraiinsPool(serde_json::from_value(json).map_err(E::custom)?)
            }
            "formula-1" => DeviceParams::Formula1(serde_json::from_value(json).map_err(E::custom)?),
            other => return Err(de::Error::unknown_variant(other, DEVICE_KEYS)),
        })
    }

    fn fallback<E: de::Error>(device: &str) -> Result<Self, E> {
        Ok(match device {
            "bos" => DeviceParams::Bos(bos::Params::default()),
            "bos-libre" => DeviceParams::Ubos(ubos::Params::default()),
            "axeos" => DeviceParams::Axeos(axeos::Params::default()),
            "braiins-pool" => DeviceParams::BraiinsPool(braiins_pool::Params::default()),
            "formula-1" => DeviceParams::Formula1(formula_1::Params::default()),
            other => return Err(de::Error::unknown_variant(other, DEVICE_KEYS)),
        })
    }

    fn into_instance(self, label: Option<String>, count: usize, port: Option<u16>) -> Instance {
        match self {
            DeviceParams::Bos(params) => Instance::Bos {
                label,
                params,
                count,
                port,
            },
            DeviceParams::Ubos(params) => Instance::Ubos {
                label,
                params,
                count,
                port,
            },
            DeviceParams::Axeos(params) => Instance::Axeos {
                label,
                params,
                count,
                port,
            },
            DeviceParams::BraiinsPool(params) => Instance::BraiinsPool {
                label,
                params,
                count,
                port,
            },
            DeviceParams::Formula1(params) => Instance::Formula1 {
                label,
                params,
                count,
                port,
            },
        }
    }
}

/// Hand-written rather than derived: `#[serde(tag = ...)]` makes serde buffer
/// each entry into an in-memory `Content` before dispatching on the tag, and a
/// value rejected inside that buffer carries no source position — which costs
/// every blueprint error its caret. Reading the tag first and the params
/// straight from the map keeps the format's spans intact.
impl<'de> Deserialize<'de> for Instance {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(InstanceVisitor)
    }
}

struct InstanceVisitor;

impl<'de> Visitor<'de> for InstanceVisitor {
    type Value = Instance;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a device instance naming its `device`")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Instance, A::Error> {
        let mut device: Option<String> = None;
        let mut label: Option<Option<String>> = None;
        let mut count: Option<usize> = None;
        let mut port: Option<Option<u16>> = None;
        let mut params: Option<DeviceParams> = None;
        let mut early_params: Option<Json> = None;

        while let Some(key) = map.next_key::<String>()? {
            // Refuse a repeated key as the derive would; a duplicated `params`
            // block is a copy-paste whose earlier half would vanish unread.
            match key.as_str() {
                "device" => {
                    if device.is_some() {
                        return Err(de::Error::duplicate_field("device"));
                    }
                    device = Some(map.next_value()?);
                }
                "label" => {
                    if label.is_some() {
                        return Err(de::Error::duplicate_field("label"));
                    }
                    label = Some(map.next_value()?);
                }
                "count" => {
                    if count.is_some() {
                        return Err(de::Error::duplicate_field("count"));
                    }
                    count = Some(map.next_value()?);
                }
                "port" => {
                    if port.is_some() {
                        return Err(de::Error::duplicate_field("port"));
                    }
                    port = Some(map.next_value()?);
                }
                "params" => {
                    if params.is_some() || early_params.is_some() {
                        return Err(de::Error::duplicate_field("params"));
                    }
                    match device.as_deref() {
                        Some(device) => params = Some(DeviceParams::read(device, &mut map)?),
                        None => early_params = Some(map.next_value()?),
                    }
                }
                other => return Err(de::Error::unknown_field(other, INSTANCE_FIELDS)),
            }
        }

        let device = device.ok_or_else(|| de::Error::missing_field("device"))?;
        let params = match (params, early_params) {
            (Some(params), _) => params,
            (None, Some(json)) => DeviceParams::reparse(&device, json)?,
            (None, None) => DeviceParams::fallback(&device)?,
        };
        Ok(params.into_instance(label.flatten(), count.unwrap_or_else(one), port.flatten()))
    }
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
            Instance::BraiinsPool { .. } => "braiins-pool",
            Instance::Formula1 { .. } => "formula-1",
        }
    }

    /// How many copies of this instance to bring up.
    #[must_use]
    pub fn count(&self) -> usize {
        match self {
            Instance::Bos { count, .. }
            | Instance::Ubos { count, .. }
            | Instance::Axeos { count, .. }
            | Instance::BraiinsPool { count, .. }
            | Instance::Formula1 { count, .. } => *count,
        }
    }

    /// The entry's human scenario label, if the blueprint set one.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        match self {
            Instance::Bos { label, .. }
            | Instance::Ubos { label, .. }
            | Instance::Axeos { label, .. }
            | Instance::BraiinsPool { label, .. }
            | Instance::Formula1 { label, .. } => label.as_deref(),
        }
    }

    /// The entry's pinned base port, if the blueprint set one.
    #[must_use]
    pub fn port(&self) -> Option<u16> {
        match self {
            Instance::Bos { port, .. }
            | Instance::Ubos { port, .. }
            | Instance::Axeos { port, .. }
            | Instance::BraiinsPool { port, .. }
            | Instance::Formula1 { port, .. } => *port,
        }
    }

    /// Build one concrete resource from this instance's params.
    #[must_use]
    pub fn resource(&self, name: &str, port: u16) -> ResourceSpec {
        match self {
            Instance::Bos { params, .. } => params.resource(name, port),
            Instance::Ubos { params, .. } => params.resource(name, port),
            Instance::Axeos { params, .. } => params.resource(name, port),
            Instance::BraiinsPool { params, .. } => params.resource(name, port),
            Instance::Formula1 { params, .. } => params.resource(name, port),
        }
    }
}

/// A fully-resolved resource ready to advertise and serve.
#[derive(Debug, Clone)]
pub struct ResourceSpec {
    pub name: String,
    pub port: u16,
    /// `None` for a cloud-API resource: it is reached by its port rather
    /// than discovered on the LAN, so nothing is advertised.
    pub announce: Option<AnnounceSpec>,
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
    pub status: HttpStatus,
}

/// How an endpoint produces its response body.
#[derive(Clone)]
pub enum Body {
    /// A JSON template whose `$value` leaves are rendered per request.
    Render(Json),
    /// A response built from the device's accumulated cache (history).
    Accumulate(AccumFn),
    /// A response computed from the request itself — for endpoints keyed
    /// on their query string (windowed history, cursor pagination).
    Respond(RespondFn),
    /// A fixed non-JSON payload, for the binaries an API hands out
    /// alongside its JSON — images above all.
    Bytes {
        content_type: String,
        data: Arc<[u8]>,
    },
}

/// Reads a device's cache and shapes it into an endpoint response body.
pub type AccumFn = Arc<dyn Fn(&Cache) -> Json + Send + Sync>;

/// Shapes a request-aware endpoint's response body.
pub type RespondFn = Arc<dyn Fn(&RequestCtx) -> Json + Send + Sync>;

/// What a [`Body::Respond`] endpoint sees of its request and device.
#[derive(Debug, Clone)]
pub struct RequestCtx {
    /// The parsed query string, later duplicates winning.
    pub query: BTreeMap<String, String>,
    /// Elapsed scenario time in seconds.
    pub t_s: f64,
    /// The device's noise seed.
    pub seed: u64,
    /// The request's `Host`, absent when the client sent none.
    ///
    /// A payload pointing at the simulator's own binaries must name an address
    /// the caller can dial — which the simulator cannot know:
    /// it is `localhost` to the testbed and a LAN address to a deck.
    /// Echoing back the host that reached us answers both, with no knob to set.
    /// It is client-controlled, so a served product could not reflect it back;
    /// a simulator on a dev LAN has no attacker to hand it one.
    pub host: Option<String>,
}

impl Body {
    /// Wrap a cache reader as an accumulating endpoint body.
    #[must_use]
    pub fn accumulate<F>(reader: F) -> Self
    where
        F: Fn(&Cache) -> Json + Send + Sync + 'static,
    {
        Body::Accumulate(Arc::new(reader))
    }

    /// Wrap a request reader as a query-aware endpoint body.
    #[must_use]
    pub fn respond<F>(responder: F) -> Self
    where
        F: Fn(&RequestCtx) -> Json + Send + Sync + 'static,
    {
        Body::Respond(Arc::new(responder))
    }
}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Body::Render(template) => f.debug_tuple("Render").field(template).finish(),
            Body::Accumulate(_) => f.debug_tuple("Accumulate").finish_non_exhaustive(),
            Body::Respond(_) => f.debug_tuple("Respond").finish_non_exhaustive(),
            Body::Bytes { content_type, data } => f
                .debug_struct("Bytes")
                .field("content_type", content_type)
                .field("len", &data.len())
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Blueprint, Instance};

    fn instance(source: &str) -> Instance {
        json5::from_str(source).expect("BUG: instance must parse")
    }

    #[test]
    fn reads_each_device_key_into_its_variant() {
        assert!(matches!(
            instance(r#"{ device: "bos" }"#),
            Instance::Bos { .. }
        ));
        assert!(matches!(
            instance(r#"{ device: "bos-libre" }"#),
            Instance::Ubos { .. }
        ));
        assert!(matches!(
            instance(r#"{ device: "axeos" }"#),
            Instance::Axeos { .. }
        ));
    }

    #[test]
    fn omitted_params_and_count_fall_back_to_defaults() {
        let Instance::Bos { label, count, .. } = instance(r#"{ device: "bos" }"#) else {
            panic!("BUG: expected a BOS instance");
        };
        assert_eq!(count, 1, "count defaults to one");
        assert_eq!(label, None);
    }

    #[test]
    fn params_before_device_still_load() {
        // The span is lost for this ordering, but the blueprint must not break.
        let Instance::Bos { params, .. } =
            instance(r#"{ params: { uptime_s: 42 }, device: "bos" }"#)
        else {
            panic!("BUG: expected a BOS instance");
        };
        assert_eq!(params.uptime_s, 42);
    }

    #[test]
    fn a_rejected_param_keeps_its_source_position() {
        // The whole reason `Deserialize` is hand-written: a derived, internally
        // tagged enum buffers the entry and loses this location.
        let err = json5::from_str::<Instance>(
            "{\n  device: \"bos\",\n  params: {\n    status: 99,\n  },\n}",
        )
        .expect_err("BUG: 99 must be rejected");
        let json5::Error::Message { location, .. } = err;
        let location = location.expect("BUG: the rejection must carry a location");
        assert_eq!(location.line, 4, "the status sits on the fourth line");
    }

    #[test]
    fn an_unknown_device_names_the_ones_that_exist() {
        let err = json5::from_str::<Instance>(r#"{ device: "toaster" }"#)
            .expect_err("BUG: unknown device must be rejected");
        let json5::Error::Message { msg, .. } = err;
        assert!(msg.contains("toaster"), "message was: {msg}");
        assert!(msg.contains("bos-libre"), "must list the valid keys: {msg}");
    }

    #[test]
    fn a_missing_device_is_rejected() {
        assert!(json5::from_str::<Instance>(r"{ count: 2 }").is_err());
    }

    #[test]
    fn a_repeated_key_is_refused() {
        let err = json5::from_str::<Instance>(
            r#"{ device: "bos", params: { uptime_s: 1 }, params: { uptime_s: 2 } }"#,
        )
        .expect_err("BUG: a duplicated params block must be refused");
        let json5::Error::Message { msg, .. } = err;
        assert!(msg.contains("params"), "names the repeated key: {msg}");
    }

    #[test]
    fn an_unknown_field_is_refused() {
        // A blueprint is hand-authored input, not persisted state: a mistyped
        // key would silently drop the fault it meant to inject.
        let err = json5::from_str::<Instance>(r#"{ device: "bos", stauts: 503 }"#)
            .expect_err("BUG: a mistyped key must be refused");
        let json5::Error::Message { msg, .. } = err;
        assert!(msg.contains("stauts"), "names the offender: {msg}");
    }

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
