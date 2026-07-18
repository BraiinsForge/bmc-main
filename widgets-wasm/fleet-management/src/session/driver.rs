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

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use bmc_wasm_sdk::profile;
use bmc_wasm_sdk::ufmt;
use bmc_wasm_sdk::{
    FetchRequest, FetchRequestId, SystemTime, fmt, format_number, log_debug, log_info, log_warn,
    request_frame,
};

use super::{
    EndpointOutcome, PassCursor, ReauthDecision, adapter_for, pass_reachable, reauth_decision,
};
use crate::adapter::FamilyAdapter;
use crate::device::{DeviceFamily, DeviceId, PollFailure, family_label};
use crate::families::bos::{self, BosAdapter};
use crate::manifest_params::Params;
use crate::model::{MinerModel, ModelAccumulator};
use crate::telemetry::TelemetryReading;

// One global round-robin walks every pollable device in a single rotation,
// with only one request-cycle in flight at a time. Each device's opening
// fetch is deferred by `tick_ms`, so completions arrive as an even drip
// instead of a per-family burst — and the spread doubles as a load cap
// on a weak device: the `MIN_TICK_MS` floor bounds the poll rate,
// so a large fleet's per-device freshness degrades gracefully
// rather than the box drowning in parallel work.

/// Target time to refresh every device once, spread across the rotation.
const TARGET_PERIOD_MS: u32 = 5_000;
/// Floor on the gap between device polls: at most `1000 / MIN_TICK_MS`
/// device polls per second, however many devices there are.
const MIN_TICK_MS: u32 = 150;
// Fleet devices live on the local network, so an unreachable one should fail
// fast instead of holding the SDK-default 10s timeout.
const DEVICE_FETCH_TIMEOUT: Duration = Duration::from_secs(1);
// Drop a no-response device from the fleet after this long unreachable,
// so a dead fleet's count decays. Generous, so a rebooting miner recovers first.
const RETIRE_AFTER_SECS: i64 = 300;

/// The gap before the next device's opening fetch: the rotation period
/// split evenly across the ring, floored at [`MIN_TICK_MS`].
fn tick_ms(ring_len: usize) -> u32 {
    let n = u32::try_from(ring_len).unwrap_or(u32::MAX).max(1);
    (TARGET_PERIOD_MS / n).max(MIN_TICK_MS)
}

enum FetchKind {
    Login,
    Telemetry { endpoint_idx: usize },
}

struct InFlight {
    device: DeviceId,
    generation: u64,
    kind: FetchKind,
}

/// A base-type BOS candidate awaiting its version fingerprint.
/// `doc` is the original Found payload, re-ingested once the probe confirms BOS.
struct ProbePending {
    id: DeviceId,
    doc: String,
}

/// The single global poller. `ring` is the current rotation snapshot;
/// only the device at its cursor is ever in flight, so the accumulators
/// below all belong to that one device.
///
/// `active` holds while a rotation is running (a fetch is in flight
/// or its opening fetch is scheduled) and clears when the ring rebuilds empty.
///
/// `current_family` is the in-flight device's family, kept for logging.
struct Poller {
    ring: PassCursor,
    /// The ring's device count, for sizing the inter-poll tick.
    ring_len: usize,
    active: bool,
    current_family: DeviceFamily,
    generation: u64,
    pending: usize,
    reading: TelemetryReading,
    model: ModelAccumulator,
    outcomes: Vec<Option<EndpointOutcome>>,
    reauthed: bool,
    /// Why this device's endpoints failed so far, for its surfaced status
    /// when it yields no telemetry. Reset per device; `ApiError` dominates a mix.
    pass_failure: Option<PollFailure>,
}

impl Poller {
    fn idle() -> Self {
        Self {
            ring: PassCursor::new(Vec::new()),
            ring_len: 0,
            active: false,
            current_family: DeviceFamily::Bos,
            generation: 0,
            pending: 0,
            reading: TelemetryReading::default(),
            model: ModelAccumulator::default(),
            outcomes: Vec::new(),
            reauthed: false,
            pass_failure: None,
        }
    }

    fn current(&self) -> Option<DeviceId> {
        self.ring.current().cloned()
    }
}

thread_local! {
    static POLLER: RefCell<Poller> = RefCell::new(Poller::idle());
    static TOKENS: RefCell<HashMap<DeviceId, String>> = RefCell::new(HashMap::new());
    static ROUTES: RefCell<HashMap<FetchRequestId, InFlight>> = RefCell::new(HashMap::new());
    static PROBES: RefCell<HashMap<FetchRequestId, ProbePending>> = RefCell::new(HashMap::new());
    static PROBING: RefCell<HashSet<DeviceId>> = RefCell::new(HashSet::new());
    static PARAMS: RefCell<Rc<Params>> = RefCell::new(Rc::new(Params::current()));
}

/// The operator params last seen by the driver. Cached as an `Rc` so the hot
/// fetch paths clone a pointer instead of every param String; refreshed only
/// when `on_params_update` fires.
fn params() -> Rc<Params> {
    PARAMS.with(|p| Rc::clone(&p.borrow()))
}

/// Re-read the param snapshot into the driver cache. Called from
/// `on_params_update`.
pub fn refresh_params() {
    PARAMS.with(|p| *p.borrow_mut() = Rc::new(Params::current()));
}

fn with_poller<R>(f: impl FnOnce(&mut Poller) -> R) -> R {
    POLLER.with(|p| f(&mut p.borrow_mut()))
}

pub fn family_enabled(family: DeviceFamily) -> bool {
    match family {
        DeviceFamily::Bos | DeviceFamily::Ubos => true,
        DeviceFamily::Bitaxe => params().axeos_enabled,
    }
}

fn base_url(adapter: &dyn FamilyAdapter, host: &str, port: u16) -> String {
    fmt!("http://{}:{}{}", host, port, adapter.api_base_path())
}

fn resolve_identity(id: &DeviceId) -> Option<(DeviceFamily, String, u16)> {
    let _s = profile::span("resolve");
    crate::DEVICES.with(|devs| {
        let devs = devs.borrow();
        let dev = devs.iter().find(|d| &d.identity.id == id)?;
        Some((
            dev.identity.family,
            dev.identity.host.clone(),
            dev.identity.port,
        ))
    })
}

/// Every pollable device across the enabled families, in family order
/// — the snapshot the ring rebuilds from at the start of each rotation.
fn gather_ring() -> Vec<DeviceId> {
    let _s = profile::span("gather_ring");
    let now = SystemTime::now().unix_secs;
    let retired = crate::DEVICES.with(|d| d.borrow_mut().prune_gone(now, RETIRE_AFTER_SECS));
    if retired > 0 {
        log_info!("fleet: retired {retired} device(s) unreachable > {RETIRE_AFTER_SECS}s");
        crate::request_display_frame();
    }
    let mut ids = Vec::new();
    for family in DeviceFamily::ALL {
        if family_enabled(family) {
            ids.extend(crate::DEVICES.with(|d| d.borrow().pollable_ids_for_family(family)));
        }
    }
    // Debug-level so it's silent at the production INFO threshold; raise the
    // device log level to DEBUG to watch the fleet's membership over time.
    let census = crate::DEVICES.with(|d| d.borrow().census());
    log_debug!(
        "fleet census: total={} reported={} reachable={} ring={} — cand={} dormant={} ident={} confirmed={}",
        census.total,
        census.reported,
        census.reachable,
        ids.len(),
        census.candidate,
        census.dormant,
        census.identified,
        census.confirmed,
    );
    ids
}

/// Send `req` now, or after `delay_ms` when spacing
/// the next device's opening fetch across the rotation.
fn send(req: FetchRequest<'_>, delay_ms: u32) -> Option<FetchRequestId> {
    if delay_ms == 0 {
        req.send(on_fetch)
    } else {
        req.send_after(delay_ms, on_fetch)
    }
}

/// The unauthenticated endpoint BOS answers on the shared `_http._tcp` browse,
/// appended to the API base — so the full probe URL is `…/api/v1/version`.
const BOS_PROBE_ENDPOINT: &str = "/version";

/// Fingerprint a base-type BOS candidate before it can be sent credentials.
///
/// BOS shares `_http._tcp` with arbitrary hosts, so a fresh sighting is probed
/// over the unauthenticated `/version` endpoint and ingested only if it answers
/// BOS-shaped; a host already in the fleet just refreshes. The gate filters
/// benign hosts (printers, NAS) — not proof against deliberate impersonation.
pub fn probe_bos_candidate(json: &str) {
    let doc = bmc_wasm_sdk::JsonDoc::parse(json.as_bytes());
    let Some(found) = BosAdapter.parse_found(&doc) else {
        return;
    };
    let id = found.identity.id;
    // A device already in the fleet was fingerprinted on first sighting; just refresh.
    let known = crate::DEVICES.with(|d| d.borrow().iter().any(|dev| dev.identity.id == id));
    if known {
        crate::ingest_probed_bos(json);
        return;
    }
    // One probe per candidate at a time, so re-announcements don't pile on.
    if PROBING.with(|p| !p.borrow_mut().insert(id.clone())) {
        return;
    }
    let url = fmt!(
        "{}{}",
        base_url(&BosAdapter, &found.identity.host, found.identity.port),
        BOS_PROBE_ENDPOINT,
    );
    let sent = FetchRequest::get(&url)
        .timeout(DEVICE_FETCH_TIMEOUT)
        .send(on_probe);
    if let Some(req_id) = sent {
        PROBES.with(|r| {
            r.borrow_mut().insert(
                req_id,
                ProbePending {
                    id,
                    doc: json.to_owned(),
                },
            );
        });
    } else {
        PROBING.with(|p| p.borrow_mut().remove(&id));
    }
}

fn on_probe(response: &bmc_wasm_sdk::FetchResponse) {
    let Some(pending) = PROBES.with(|r| r.borrow_mut().remove(&response.request_id)) else {
        return;
    };
    PROBING.with(|p| p.borrow_mut().remove(&pending.id));
    if response.ok() && bos::is_version_response(&response.json()) {
        crate::ingest_probed_bos(&pending.doc);
    } else {
        log_debug!(
            "fleet: {} failed the BOS fingerprint — not sending credentials",
            pending.id.as_str()
        );
    }
}

/// Ensure the rotation is running. A no-op while one is already active;
/// from idle it snapshots the ring and polls the first device at once,
/// so a freshly discovered device gets data promptly.
///
/// The family/`is_new` no longer matter — the global ring picks up
/// every pollable device — but the signature is kept so `lib.rs`
/// can keep calling it per discovery, re-enable, or manual-host change.
pub fn on_discovered(_family: DeviceFamily, _is_new: bool) {
    kick();
}

pub fn ensure_running(_family: DeviceFamily) {
    kick();
}

fn kick() {
    if with_poller(|p| p.active) {
        return;
    }
    let ring = gather_ring();
    if ring.is_empty() {
        return;
    }
    let ring_len = ring.len();
    with_poller(|p| {
        p.ring = PassCursor::new(ring);
        p.ring_len = ring_len;
        p.active = true;
    });
    begin_device(0);
}

/// Drop cached session tokens for one family's devices (e.g. after that family's
/// credentials changed), forcing a fresh login on their next poll.
/// Other families' tokens are left intact.
pub fn clear_tokens_for(family: DeviceFamily) {
    let ids = crate::DEVICES.with(|d| d.borrow().ids_for_family(family));
    TOKENS.with(|t| {
        let mut tokens = t.borrow_mut();
        for id in &ids {
            tokens.remove(id);
        }
    });
}

/// Stop polling a family (the operator disabled it).
/// Its devices drop out of the ring at the next rebuild
/// and are skipped at their turn meanwhile; if one is the in-flight device,
/// abandon it so the rotation moves on at once.
pub fn stop(family: DeviceFamily) {
    if with_poller(|p| p.active) && current_family_is(family) {
        abandon_current();
    }
}

fn current_family_is(family: DeviceFamily) -> bool {
    with_poller(|p| p.current())
        .and_then(|id| resolve_identity(&id))
        .is_some_and(|(f, _, _)| f == family)
}

/// Drop one device's cached session state; if it is the in-flight device,
/// abandon it so the rotation advances. Otherwise it is skipped
/// at its turn (its identity no longer resolves) or simply absent
/// from the next rebuild.
pub fn remove_token(id: &DeviceId) {
    TOKENS.with(|t| t.borrow_mut().remove(id));
    if with_poller(|p| p.current().as_ref() == Some(id)) {
        abandon_current();
    }
}

/// Begin the device at the ring cursor after `delay_ms`,
/// skipping any that have left the fleet or a disabled family.
/// Rebuilds the ring when the rotation wraps;
/// a rebuild that finds nothing pollable parks the poller idle.
fn begin_device(delay_ms: u32) {
    let mut rebuilt = false;
    loop {
        if with_poller(|p| p.ring.is_done()) {
            if rebuilt {
                // The fresh ring was already all-unpollable this call; go idle.
                park_idle();
                return;
            }
            let ring = gather_ring();
            if ring.is_empty() {
                park_idle();
                return;
            }
            let ring_len = ring.len();
            with_poller(|p| {
                p.ring = PassCursor::new(ring);
                p.ring_len = ring_len;
            });
            rebuilt = true;
        }
        let Some(id) = with_poller(|p| p.current()) else {
            park_idle();
            return;
        };
        let Some((dev_family, host, port)) = resolve_identity(&id) else {
            with_poller(|p| p.ring.advance()); // device left the fleet
            continue;
        };
        if !family_enabled(dev_family) {
            with_poller(|p| p.ring.advance()); // family disabled mid-rotation
            continue;
        }
        let Some(adapter) = adapter_for(dev_family) else {
            log_warn!("fleet: no adapter for discovered device family; marking unreachable");
            crate::DEVICES.with(|devs| {
                devs.borrow_mut()
                    .record_pass(&id, TelemetryReading::default(), false);
            });
            request_frame();
            with_poller(|p| p.ring.advance());
            continue;
        };
        let endpoint_count = adapter.telemetry_endpoints().len();
        with_poller(|p| {
            p.current_family = dev_family;
            p.generation = p.generation.wrapping_add(1);
            p.pending = 0;
            p.reading = TelemetryReading::default();
            p.model = ModelAccumulator::default();
            p.outcomes = vec![None; endpoint_count];
            p.reauthed = false;
            p.pass_failure = None;
        });
        let needs_login =
            adapter.auth_endpoint().is_some() && TOKENS.with(|t| !t.borrow().contains_key(&id));
        if needs_login {
            issue_login(&id, dev_family, &host, port, adapter, delay_ms);
        } else {
            fire_pending(&id, dev_family, &host, port, adapter, delay_ms);
        }
        return;
    }
}

fn park_idle() {
    with_poller(|p| {
        p.active = false;
        p.ring = PassCursor::new(Vec::new());
        p.ring_len = 0;
    });
}

/// Fire every still-pending (`None`) endpoint of the current device.
/// `delay_ms` spaces the opening fetch across the rotation;
/// the rest go out at once.
fn fire_pending(
    id: &DeviceId,
    family: DeviceFamily,
    host: &str,
    port: u16,
    adapter: &dyn FamilyAdapter,
    delay_ms: u32,
) {
    let endpoints = adapter.telemetry_endpoints();
    let params = params();
    let token = TOKENS.with(|t| t.borrow().get(id).cloned());
    let header = adapter
        .credential_header(&params.ubos_username, &params.ubos_password)
        .or_else(|| token.map(|t| adapter.auth_header(&t)));
    let generation = with_poller(|p| p.generation);
    let pending_idxs: Vec<usize> = with_poller(|p| {
        p.outcomes
            .iter()
            .enumerate()
            .filter(|(_, o)| o.is_none())
            .map(|(i, _)| i)
            .collect()
    });
    log_info!(
        "fleet: {} {} fetching telemetry at {}",
        family_label(family),
        id.as_str(),
        base_url(adapter, host, port),
    );
    let mut sent = 0_usize;
    for idx in pending_idxs {
        let url = fmt!("{}{}", base_url(adapter, host, port), endpoints[idx]);
        let req_id = send(
            FetchRequest::get(&url)
                .headers_opt(header.as_deref())
                .timeout(DEVICE_FETCH_TIMEOUT),
            delay_ms,
        );
        if let Some(req_id) = req_id {
            ROUTES.with(|r| {
                r.borrow_mut().insert(
                    req_id,
                    InFlight {
                        device: id.clone(),
                        generation,
                        kind: FetchKind::Telemetry { endpoint_idx: idx },
                    },
                );
            });
            sent += 1;
        } else {
            log_warn!("fleet: telemetry send rejected for {}", host);
            with_poller(|p| p.outcomes[idx] = Some(EndpointOutcome::Failed));
        }
    }
    with_poller(|p| p.pending = sent);
    if sent == 0 {
        barrier();
    }
}

fn issue_login(
    id: &DeviceId,
    family: DeviceFamily,
    host: &str,
    port: u16,
    adapter: &dyn FamilyAdapter,
    delay_ms: u32,
) {
    let Some(auth_path) = adapter.auth_endpoint() else {
        finalize_failed(id);
        return;
    };
    let params = params();
    let url = fmt!("{}{}", base_url(adapter, host, port), auth_path);
    log_info!(
        "fleet: {} {} logging in at {}",
        family_label(family),
        id.as_str(),
        url,
    );
    let body = adapter.login_body(&params.bos_password);
    let generation = with_poller(|p| p.generation);
    let req = FetchRequest::post(&url)
        .headers("Content-Type: application/json")
        .body(body.as_bytes())
        .timeout(DEVICE_FETCH_TIMEOUT);
    if let Some(req_id) = send(req, delay_ms) {
        ROUTES.with(|r| {
            r.borrow_mut().insert(
                req_id,
                InFlight {
                    device: id.clone(),
                    generation,
                    kind: FetchKind::Login,
                },
            );
        });
    } else {
        log_warn!("fleet: login send rejected for {}", host);
        finalize_failed(id);
    }
}

/// Shared callback for every login and telemetry fetch.
/// Drops any response whose `(device, generation)` no longer
/// matches the in-flight device — the rotation abandoned it or has moved on.
fn on_fetch(response: &bmc_wasm_sdk::FetchResponse) {
    let Some(route) = ROUTES.with(|r| r.borrow_mut().remove(&response.request_id)) else {
        return;
    };
    let (generation, current) = with_poller(|p| (p.generation, p.current()));
    if route.generation != generation || current.as_ref() != Some(&route.device) {
        return;
    }
    match route.kind {
        FetchKind::Login => on_login(&route.device, response),
        FetchKind::Telemetry { endpoint_idx } => on_telemetry(endpoint_idx, response),
    }
}

fn on_login(id: &DeviceId, response: &bmc_wasm_sdk::FetchResponse) {
    let Some((dev_family, host, port)) = resolve_identity(id) else {
        advance();
        return;
    };
    let Some(adapter) = adapter_for(dev_family) else {
        advance();
        return;
    };
    let token = if response.ok() {
        adapter.parse_login(&response.json())
    } else {
        None
    };
    if let Some(token) = token {
        TOKENS.with(|t| t.borrow_mut().insert(id.clone(), token));
        fire_pending(id, dev_family, &host, port, adapter, 0);
    } else {
        // 401/403, or a 200 with no token, is a rejected login — an auth failure,
        // so `finalize_device` records `AuthError` rather than retiring the device.
        // Any other answer (5xx, timeout) is transient, not a credentials problem.
        let is_auth_failure = adapter.is_auth_error(response.status) || response.ok();
        with_poller(|p| {
            for outcome in &mut p.outcomes {
                if outcome.is_none() {
                    *outcome = Some(EndpointOutcome::Failed);
                }
            }
            p.pending = 0;
            if is_auth_failure {
                p.pass_failure = Some(PollFailure::AuthError);
            }
        });
        finalize_device(id);
    }
}

fn on_telemetry(endpoint_idx: usize, response: &bmc_wasm_sdk::FetchResponse) {
    // Every exit must either complete the barrier (decrement `pending`,
    // fire on zero) or advance — otherwise a response for a device
    // that vanished mid-poll would strand `pending` and stall the rotation forever.
    let Some(id) = with_poller(|p| p.current()) else {
        advance();
        return;
    };
    let Some((dev_family, _, _)) = resolve_identity(&id) else {
        advance();
        return;
    };
    let Some(adapter) = adapter_for(dev_family) else {
        advance();
        return;
    };
    let endpoints = adapter.telemetry_endpoints();
    let Some(ep) = endpoints.get(endpoint_idx) else {
        advance();
        return;
    };

    let outcome = if adapter.auth_endpoint().is_some() && adapter.is_auth_error(response.status) {
        EndpointOutcome::AuthFailed
    } else if response.ok() {
        let _s = profile::span("parse");
        let doc = response.json();
        with_poller(|p| {
            let p = &mut *p;
            adapter.parse_telemetry(ep, &doc, &mut p.reading);
            adapter.parse_model(ep, &doc, &mut p.model);
        });
        EndpointOutcome::Ok
    } else {
        with_poller(|p| adapter.reset_telemetry(ep, &mut p.reading));
        EndpointOutcome::Failed
    };

    // Track why a failed endpoint failed, for the surfaced status of a device
    // that yields no telemetry: an HTTP status means the API answered (badly,
    // e.g. 503); status 0 means nothing reached us. `ApiError` dominates a mix —
    // any answer at all means the miner is reachable, just erroring.
    if matches!(
        outcome,
        EndpointOutcome::Failed | EndpointOutcome::AuthFailed
    ) {
        let kind = if response.status == 0 {
            PollFailure::Unreachable
        } else {
            PollFailure::ApiError
        };
        with_poller(|p| {
            p.pass_failure = match (p.pass_failure, kind) {
                (Some(PollFailure::ApiError), _) | (_, PollFailure::ApiError) => {
                    Some(PollFailure::ApiError)
                }
                _ => Some(PollFailure::Unreachable),
            };
        });
    }

    let done = with_poller(|p| {
        p.outcomes[endpoint_idx] = Some(outcome);
        p.pending = p.pending.saturating_sub(1);
        p.pending == 0
    });
    if done {
        barrier();
    }
}

/// All endpoints reported: re-authenticate once if any auth-failed, else finalize.
fn barrier() {
    let Some(id) = with_poller(|p| p.current()) else {
        advance();
        return;
    };
    let (outcomes, reauthed) = with_poller(|p| {
        let outcomes: Vec<EndpointOutcome> = p
            .outcomes
            .iter()
            .map(|o| o.unwrap_or(EndpointOutcome::Failed))
            .collect();
        (outcomes, p.reauthed)
    });
    match reauth_decision(&outcomes, reauthed) {
        ReauthDecision::Reauth { endpoints } => {
            with_poller(|p| {
                p.reauthed = true;
                for idx in &endpoints {
                    p.outcomes[*idx] = None;
                }
            });
            TOKENS.with(|t| t.borrow_mut().remove(&id));
            let Some((dev_family, host, port)) = resolve_identity(&id) else {
                advance();
                return;
            };
            let Some(adapter) = adapter_for(dev_family) else {
                advance();
                return;
            };
            issue_login(&id, dev_family, &host, port, adapter, 0);
        }
        ReauthDecision::Finalize => finalize_device(&id),
    }
}

fn finalize_device(id: &DeviceId) {
    let _s = profile::span("finalize");
    let (reading, reachable, model, pass_failure, family) = with_poller(|p| {
        let outcomes: Vec<EndpointOutcome> = p
            .outcomes
            .iter()
            .map(|o| o.unwrap_or(EndpointOutcome::Failed))
            .collect();
        (
            p.reading.clone(),
            pass_reachable(&outcomes),
            p.model.clone(),
            p.pass_failure,
            p.current_family,
        )
    });
    let model = model.into_model();
    let failures = crate::DEVICES.with(|devs| {
        let mut devs = devs.borrow_mut();
        let failures = devs.record_pass(id, reading.clone(), reachable);
        if !reachable {
            devs.set_last_failure(id, pass_failure.unwrap_or(PollFailure::Unreachable));
        }
        if let Some(model) = model.clone() {
            devs.apply_model(id, model);
        }
        failures
    });
    log_fetch(family, id, reachable, failures, &reading, model.as_ref());
    // Coalesced: a full round bumps the sequence N times,
    // but the display only needs to refold once per window,
    // not once per device.
    crate::request_display_frame();
    advance();
}

/// Report what a finished poll learned about a device: its family, model,
/// and the freshly fetched telemetry — or that the device is unreachable.
fn log_fetch(
    family: DeviceFamily,
    id: &DeviceId,
    reachable: bool,
    failures: usize,
    reading: &TelemetryReading,
    model: Option<&MinerModel>,
) {
    if !reachable {
        log_info!(
            "fleet: {} {} unreachable ({} in a row)",
            family_label(family),
            id.as_str(),
            failures,
        );
        return;
    }
    let model = model.map_or_else(|| "unknown model".to_owned(), |m| m.name.clone());
    log_info!(
        "fleet: {} {} ({}) {}",
        family_label(family),
        id.as_str(),
        model,
        telemetry_summary(reading),
    );
}

/// Compact, comma-separated rendering of the present telemetry fields, using
/// the same magnitudes the on-screen cells show.
fn telemetry_summary(reading: &TelemetryReading) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(v) = reading.current_hashrate_ths {
        parts.push(fmt!("{} TH/s", format_number!(f64::from(v), 2)));
    }
    if let Some(v) = reading.power_w {
        parts.push(fmt!("{} W", format_number!(f64::from(v), 0)));
    }
    if let Some(temp) = reading.temperature {
        let (_, avg, _) = temp.as_range();
        parts.push(fmt!("{} °C", format_number!(avg.as_celsius(), 1)));
    }
    if let Some(v) = reading.uptime_s {
        parts.push(fmt!("{} s uptime", v));
    }
    if parts.is_empty() {
        return "no telemetry".to_owned();
    }
    parts.join(", ")
}

/// Mark every endpoint failed and finalize (used when login cannot be sent).
fn finalize_failed(id: &DeviceId) {
    with_poller(|p| {
        for outcome in &mut p.outcomes {
            *outcome = Some(EndpointOutcome::Failed);
        }
        p.pending = 0;
    });
    finalize_device(id);
}

/// Abandon the in-flight device (removed, or its family disabled):
/// bump the generation so its outstanding responses are dropped, then advance.
fn abandon_current() {
    with_poller(|p| {
        p.generation = p.generation.wrapping_add(1);
        p.pending = 0;
        p.outcomes.clear();
        p.reading = TelemetryReading::default();
        p.model = ModelAccumulator::default();
        p.reauthed = false;
        p.pass_failure = None;
    });
    advance();
}

/// Advance the ring and begin the next device after a tick
/// — the gap that spreads the rotation evenly and caps the poll rate.
fn advance() {
    let tick = with_poller(|p| {
        p.ring.advance();
        tick_ms(p.ring_len)
    });
    begin_device(tick);
}
