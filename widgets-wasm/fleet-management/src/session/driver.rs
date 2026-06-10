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
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use bmc_wasm_sdk::ufmt;
use bmc_wasm_sdk::{
    FetchRequest, FetchRequestId, cancel, fmt, format_number, log_info, log_warn, request_frame,
};

use super::{
    DiscoveryAction, EndpointOutcome, PassCursor, Phase, ReauthDecision, RemovalAction,
    adapter_for, on_discovery, on_removal, pass_reachable, reauth_decision,
};
use crate::adapter::FamilyAdapter;
use crate::device::{DeviceFamily, DeviceId, FamilyMap, family_label};
use crate::manifest_params::Params;
use crate::model::{MinerModel, ModelAccumulator};
use crate::telemetry::TelemetryReading;

const PASS_INTERVAL_MS: u32 = 15_000;
// Fleet devices live on the local network, so an unreachable one should fail
// fast instead of holding the SDK-default 10s timeout for a whole pass.
const DEVICE_FETCH_TIMEOUT: Duration = Duration::from_secs(1);

enum FetchKind {
    Login,
    Telemetry { endpoint_idx: usize },
}

struct InFlight {
    family: DeviceFamily,
    device: DeviceId,
    generation: u64,
    kind: FetchKind,
}

struct FamilyDriver {
    cursor: Option<PassCursor>,
    /// Opening fetches of the next pass, scheduled via `send_after` and not yet
    /// delivered. Non-empty marks the `Waiting` phase; the ids let a discovery
    /// or removal cancel the kick to react before the interval elapses.
    pending_kick: Vec<FetchRequestId>,
    generation: u64,
    pending: usize,
    reading: TelemetryReading,
    model: ModelAccumulator,
    outcomes: Vec<Option<EndpointOutcome>>,
    reauthed: bool,
}

impl FamilyDriver {
    fn idle() -> Self {
        Self {
            cursor: None,
            pending_kick: Vec::new(),
            generation: 0,
            pending: 0,
            reading: TelemetryReading::default(),
            model: ModelAccumulator::default(),
            outcomes: Vec::new(),
            reauthed: false,
        }
    }

    fn current_device(&self) -> Option<DeviceId> {
        self.cursor.as_ref()?.current().cloned()
    }

    fn phase(&self) -> Phase {
        if !self.pending_kick.is_empty() {
            Phase::Waiting
        } else if self.cursor.is_some() {
            Phase::Active
        } else {
            Phase::Idle
        }
    }
}

thread_local! {
    static DRIVERS: RefCell<FamilyMap<FamilyDriver>> =
        RefCell::new(FamilyMap::from_fn(|_| FamilyDriver::idle()));
    static TOKENS: RefCell<HashMap<DeviceId, String>> = RefCell::new(HashMap::new());
    static ROUTES: RefCell<HashMap<FetchRequestId, InFlight>> = RefCell::new(HashMap::new());
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

fn with_driver<R>(family: DeviceFamily, f: impl FnOnce(&mut FamilyDriver) -> R) -> R {
    DRIVERS.with(|d| f(&mut d.borrow_mut()[family]))
}

pub fn family_enabled(family: DeviceFamily) -> bool {
    let params = params();
    match family {
        DeviceFamily::Bos => params.bos_enabled,
        DeviceFamily::Ubos => params.ubos_enabled,
        DeviceFamily::Bitaxe => params.axeos_enabled,
    }
}

fn base_url(adapter: &dyn FamilyAdapter, host: &str, port: u16) -> String {
    fmt!("http://{}:{}{}", host, port, adapter.api_base_path())
}

fn resolve_identity(id: &DeviceId) -> Option<(DeviceFamily, String, u16)> {
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

/// Send `req` now, or after `delay_ms` when deferring a pass's opening fetch.
fn send(req: FetchRequest<'_>, delay_ms: u32) -> Option<FetchRequestId> {
    if delay_ms == 0 {
        req.send(on_fetch)
    } else {
        req.send_after(delay_ms, on_fetch)
    }
}

/// Cancel every still-queued opening fetch for `family`, returning whether the
/// whole kick was caught before firing. A single `false` means one fetch is
/// already in flight and will drive the pass, so the caller must not start a
/// competing one. Clears the parked kick state and routes either way.
fn cancel_kick(family: DeviceFamily) -> bool {
    let ids = with_driver(family, |d| std::mem::take(&mut d.pending_kick));
    let mut all_caught = true;
    for id in ids {
        if cancel(id) {
            // Caught before firing: drop its route. A cancelled request's
            // callback never runs, so a telemetry kick that counted it in
            // `pending` must be decremented here — otherwise a surviving
            // in-flight sibling can never bring `pending` to zero and the
            // barrier (and the whole family) stalls.
            let route = ROUTES.with(|r| r.borrow_mut().remove(&id));
            if matches!(
                route,
                Some(InFlight {
                    kind: FetchKind::Telemetry { .. },
                    ..
                })
            ) {
                with_driver(family, |d| d.pending = d.pending.saturating_sub(1));
            }
        } else {
            all_caught = false;
        }
    }
    all_caught
}

/// React to a discovery for `family`; `is_new` is whether it added a device not
/// already listed. An idle family starts a pass now. A genuinely new device that
/// arrives while a kick is parked cancels the kick and restarts so it is polled
/// immediately instead of after the interval; a re-announcement of an
/// already-known device leaves the parked kick alone so the poll cadence holds
/// instead of collapsing into a back-to-back pass.
pub fn on_discovered(family: DeviceFamily, is_new: bool) {
    let phase = with_driver(family, |d| d.phase());
    let kick_cancelled = match phase {
        Phase::Waiting if is_new => Some(cancel_kick(family)),
        Phase::Idle | Phase::Waiting | Phase::Active => None,
    };
    match on_discovery(phase, is_new, kick_cancelled) {
        DiscoveryAction::StartNow => start_pass(family, 0),
        DiscoveryAction::LetRun | DiscoveryAction::Ignore => {}
    }
}

/// Ensure a pass is running for `family` after a resume (the family was
/// re-enabled or credentials changed) or after manual hosts were added — start
/// promptly, as for a newly discovered device.
pub fn ensure_running(family: DeviceFamily) {
    on_discovered(family, true);
}

/// Drop cached session tokens for one family's devices (e.g. after that
/// family's credentials changed), forcing a fresh login on the next pass.
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

/// Stop polling a family, e.g. when the operator disables it. Cancels any
/// queued opening fetch, bumps the generation so responses still in flight are
/// dropped, and clears the cursor so no further devices are polled. mDNS
/// discovery keeps running and the devices stay listed; re-enabling the family
/// starts a fresh pass via `ensure_running`.
pub fn stop(family: DeviceFamily) {
    cancel_kick(family);
    with_driver(family, |d| {
        d.generation = d.generation.wrapping_add(1);
        d.pending = 0;
        d.cursor = None;
        d.outcomes.clear();
        d.reading = TelemetryReading::default();
        d.model = ModelAccumulator::default();
        d.reauthed = false;
    });
}

/// Drop one device's cached session state and react to its departure: abandon
/// the in-flight pass if it is the current device, or cancel and re-defer the
/// next pass if its opening kick is parked.
pub fn remove_token(id: &DeviceId) {
    TOKENS.with(|t| t.borrow_mut().remove(id));
    for family in DeviceFamily::ALL {
        let phase = with_driver(family, |d| d.phase());
        let is_focus = with_driver(family, |d| d.current_device().as_ref() == Some(id));
        let kick_cancelled = if phase == Phase::Waiting && is_focus {
            Some(cancel_kick(family))
        } else {
            None
        };
        match on_removal(phase, is_focus, kick_cancelled) {
            RemovalAction::Abandon => abandon_current(family),
            RemovalAction::Redefer => start_pass(family, PASS_INTERVAL_MS),
            RemovalAction::LetRun | RemovalAction::Ignore => {}
        }
    }
}

/// Snapshot this family's devices and begin a pass. `opening_delay_ms` defers
/// only the first device's opening fetch — the inter-pass timer — while 0
/// starts immediately. An empty snapshot leaves the driver idle.
fn start_pass(family: DeviceFamily, opening_delay_ms: u32) {
    if !family_enabled(family) {
        with_driver(family, |d| {
            d.pending_kick.clear();
            d.cursor = None;
        });
        return;
    }
    let ids = crate::DEVICES.with(|d| d.borrow().ids_for_family(family));
    let empty = ids.is_empty();
    with_driver(family, |d| {
        d.pending_kick.clear();
        d.cursor = if empty {
            None
        } else {
            Some(PassCursor::new(ids))
        };
    });
    if !empty {
        begin_device(family, opening_delay_ms);
    }
}

fn begin_device(family: DeviceFamily, delay_ms: u32) {
    let done = with_driver(family, |d| {
        d.cursor.as_ref().is_none_or(PassCursor::is_done)
    });
    if done {
        // Pass finished: arm the next one as a delayed opening fetch.
        start_pass(family, PASS_INTERVAL_MS);
        return;
    }
    let Some(id) = with_driver(family, |d| d.current_device()) else {
        advance_device(family);
        return;
    };
    let Some((dev_family, host, port)) = resolve_identity(&id) else {
        advance_device(family);
        return;
    };
    let Some(adapter) = adapter_for(dev_family) else {
        log_warn!("fleet: no adapter for discovered device family; marking unreachable");
        crate::DEVICES.with(|devs| {
            devs.borrow_mut()
                .apply_telemetry(&id, TelemetryReading::default(), false);
        });
        request_frame();
        advance_device(family);
        return;
    };
    let endpoint_count = adapter.telemetry_endpoints().len();
    with_driver(family, |d| {
        d.generation = d.generation.wrapping_add(1);
        d.pending = 0;
        d.reading = TelemetryReading::default();
        d.model = ModelAccumulator::default();
        d.outcomes = vec![None; endpoint_count];
        d.reauthed = false;
    });
    let needs_login =
        adapter.auth_endpoint().is_some() && TOKENS.with(|t| !t.borrow().contains_key(&id));
    if needs_login {
        issue_login(family, &id, &host, port, adapter, delay_ms);
    } else {
        fire_pending(family, &id, &host, port, adapter, delay_ms);
    }
}

/// Fire every still-pending (`None`) endpoint of the current device. With
/// `delay_ms == 0` they go out immediately; otherwise they are the deferred
/// opening of a pass and their ids are parked as the kick.
fn fire_pending(
    family: DeviceFamily,
    id: &DeviceId,
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
    let generation = with_driver(family, |d| d.generation);
    let pending_idxs: Vec<usize> = with_driver(family, |d| {
        d.outcomes
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
                        family,
                        device: id.clone(),
                        generation,
                        kind: FetchKind::Telemetry { endpoint_idx: idx },
                    },
                );
            });
            if delay_ms > 0 {
                with_driver(family, |d| d.pending_kick.push(req_id));
            }
            sent += 1;
        } else {
            log_warn!("fleet: telemetry send rejected for {}", host);
            with_driver(family, |d| d.outcomes[idx] = Some(EndpointOutcome::Failed));
        }
    }
    with_driver(family, |d| d.pending = sent);
    if sent == 0 {
        barrier(family);
    }
}

fn issue_login(
    family: DeviceFamily,
    id: &DeviceId,
    host: &str,
    port: u16,
    adapter: &dyn FamilyAdapter,
    delay_ms: u32,
) {
    let Some(auth_path) = adapter.auth_endpoint() else {
        finalize_failed(family, id);
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
    let generation = with_driver(family, |d| d.generation);
    let req = FetchRequest::post(&url)
        .headers("Content-Type: application/json")
        .body(body.as_bytes())
        .timeout(DEVICE_FETCH_TIMEOUT);
    let req_id = send(req, delay_ms);
    if let Some(req_id) = req_id {
        ROUTES.with(|r| {
            r.borrow_mut().insert(
                req_id,
                InFlight {
                    family,
                    device: id.clone(),
                    generation,
                    kind: FetchKind::Login,
                },
            );
        });
        if delay_ms > 0 {
            with_driver(family, |d| d.pending_kick.push(req_id));
        }
    } else {
        log_warn!("fleet: login send rejected for {}", host);
        finalize_failed(family, id);
    }
}

/// Shared callback for every login and telemetry fetch. Routes by request id;
/// drops responses whose `(device, generation)` no longer matches the family's
/// current device (the pass was abandoned or has moved on).
fn on_fetch(response: &bmc_wasm_sdk::FetchResponse) {
    let Some(route) = ROUTES.with(|r| r.borrow_mut().remove(&response.request_id)) else {
        return;
    };
    let (current_generation, current_device) =
        with_driver(route.family, |d| (d.generation, d.current_device()));
    if route.generation != current_generation || current_device.as_ref() != Some(&route.device) {
        return;
    }
    // The opening fetch arrived: the pass is now active, not waiting.
    with_driver(route.family, |d| d.pending_kick.clear());
    match route.kind {
        FetchKind::Login => on_login(route.family, &route.device, response),
        FetchKind::Telemetry { endpoint_idx } => {
            on_telemetry(route.family, endpoint_idx, response);
        }
    }
}

fn on_login(family: DeviceFamily, id: &DeviceId, response: &bmc_wasm_sdk::FetchResponse) {
    let Some((dev_family, host, port)) = resolve_identity(id) else {
        advance_device(family);
        return;
    };
    let Some(adapter) = adapter_for(dev_family) else {
        advance_device(family);
        return;
    };
    let token = if response.ok() {
        adapter.parse_login(&response.json())
    } else {
        None
    };
    if let Some(token) = token {
        TOKENS.with(|t| t.borrow_mut().insert(id.clone(), token));
        fire_pending(family, id, &host, port, adapter, 0);
    } else {
        with_driver(family, |d| {
            for outcome in &mut d.outcomes {
                if outcome.is_none() {
                    *outcome = Some(EndpointOutcome::Failed);
                }
            }
            d.pending = 0;
        });
        finalize_device(family, id);
    }
}

fn on_telemetry(family: DeviceFamily, endpoint_idx: usize, response: &bmc_wasm_sdk::FetchResponse) {
    // Every exit here must either complete the barrier (decrement `pending` and
    // fire on zero) or advance the device — otherwise a telemetry response for a
    // device that vanished mid-pass would strand `pending` and leave the family
    // stuck `Active` forever. Mirror `on_login`: abandon the device on any
    // resolution failure.
    let Some(id) = with_driver(family, |d| d.current_device()) else {
        advance_device(family);
        return;
    };
    let Some((dev_family, _, _)) = resolve_identity(&id) else {
        advance_device(family);
        return;
    };
    let Some(adapter) = adapter_for(dev_family) else {
        advance_device(family);
        return;
    };
    let endpoints = adapter.telemetry_endpoints();
    let Some(ep) = endpoints.get(endpoint_idx) else {
        advance_device(family);
        return;
    };

    let outcome = if adapter.auth_endpoint().is_some() && adapter.is_auth_error(response.status) {
        EndpointOutcome::AuthFailed
    } else if response.ok() {
        let doc = response.json();
        with_driver(family, |d| {
            let d = &mut *d;
            adapter.parse_telemetry(ep, &doc, &mut d.reading);
            adapter.parse_model(ep, &doc, &mut d.model);
        });
        EndpointOutcome::Ok
    } else {
        with_driver(family, |d| adapter.reset_telemetry(ep, &mut d.reading));
        EndpointOutcome::Failed
    };

    let done = with_driver(family, |d| {
        d.outcomes[endpoint_idx] = Some(outcome);
        d.pending = d.pending.saturating_sub(1);
        d.pending == 0
    });
    if done {
        barrier(family);
    }
}

/// All endpoints reported: re-authenticate once if any auth-failed, else finalize.
fn barrier(family: DeviceFamily) {
    let Some(id) = with_driver(family, |d| d.current_device()) else {
        advance_device(family);
        return;
    };
    let (outcomes, reauthed) = with_driver(family, |d| {
        let outcomes: Vec<EndpointOutcome> = d
            .outcomes
            .iter()
            .map(|o| o.unwrap_or(EndpointOutcome::Failed))
            .collect();
        (outcomes, d.reauthed)
    });
    match reauth_decision(&outcomes, reauthed) {
        ReauthDecision::Reauth { endpoints } => {
            with_driver(family, |d| {
                d.reauthed = true;
                for idx in &endpoints {
                    d.outcomes[*idx] = None;
                }
            });
            TOKENS.with(|t| t.borrow_mut().remove(&id));
            let Some((dev_family, host, port)) = resolve_identity(&id) else {
                advance_device(family);
                return;
            };
            let Some(adapter) = adapter_for(dev_family) else {
                advance_device(family);
                return;
            };
            issue_login(family, &id, &host, port, adapter, 0);
        }
        ReauthDecision::Finalize => finalize_device(family, &id),
    }
}

fn finalize_device(family: DeviceFamily, id: &DeviceId) {
    let (reading, reachable, model) = with_driver(family, |d| {
        let outcomes: Vec<EndpointOutcome> = d
            .outcomes
            .iter()
            .map(|o| o.unwrap_or(EndpointOutcome::Failed))
            .collect();
        (d.reading, pass_reachable(&outcomes), d.model.clone())
    });
    let model = model.into_model();
    crate::DEVICES.with(|devs| {
        let mut devs = devs.borrow_mut();
        devs.apply_telemetry(id, reading, reachable);
        if let Some(model) = model.clone() {
            devs.apply_model(id, model);
        }
    });
    log_fetch(family, id, reachable, &reading, model.as_ref());
    request_frame();
    advance_device(family);
}

/// Report what a finished pass learned about a device: its family, model, and
/// the freshly fetched telemetry — or that the device is unreachable.
fn log_fetch(
    family: DeviceFamily,
    id: &DeviceId,
    reachable: bool,
    reading: &TelemetryReading,
    model: Option<&MinerModel>,
) {
    if !reachable {
        log_info!(
            "fleet: {} {} unreachable",
            family_label(family),
            id.as_str()
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
    if let Some(v) = reading.temperature_c {
        parts.push(fmt!("{} °C", format_number!(f64::from(v), 1)));
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
fn finalize_failed(family: DeviceFamily, id: &DeviceId) {
    with_driver(family, |d| {
        for outcome in &mut d.outcomes {
            *outcome = Some(EndpointOutcome::Failed);
        }
        d.pending = 0;
    });
    finalize_device(family, id);
}

/// Abandon the in-flight device (removed mid-pass): bump the generation so its
/// outstanding responses are dropped, then advance the cursor.
fn abandon_current(family: DeviceFamily) {
    with_driver(family, |d| {
        d.generation = d.generation.wrapping_add(1);
        d.pending = 0;
        d.outcomes.clear();
        d.reading = TelemetryReading::default();
        d.model = ModelAccumulator::default();
        d.reauthed = false;
        if let Some(cursor) = d.cursor.as_mut() {
            cursor.advance();
        }
    });
    begin_device(family, 0);
}

fn advance_device(family: DeviceFamily) {
    with_driver(family, |d| {
        if let Some(cursor) = d.cursor.as_mut() {
            cursor.advance();
        }
    });
    begin_device(family, 0);
}
