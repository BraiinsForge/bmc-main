// Copyright (C) 2026  Braiins Systems s.r.o.

use std::cell::RefCell;
use std::collections::HashMap;

use bmc_wasm_sdk::ufmt;
use bmc_wasm_sdk::{
    FetchRequest, FetchRequestId, fmt, log_warn, request_frame, request_frame_after,
};

use super::{
    EndpointOutcome, FamilyWake, PassCursor, ReauthDecision, adapter_for, next_wake,
    pass_reachable, reauth_decision,
};
use crate::adapter::FamilyAdapter;
use crate::device::{DeviceFamily, DeviceId};
use crate::manifest_params::Params;
use crate::model::ModelAccumulator;
use crate::telemetry::TelemetryReading;

const PASS_INTERVAL_MS: u32 = 30_000;
const FAMILIES: [DeviceFamily; 3] = [DeviceFamily::Bos, DeviceFamily::Ubos, DeviceFamily::Bitaxe];

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
    elapsed_ms: u32,
    waiting_next_pass: bool,
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
            elapsed_ms: 0,
            waiting_next_pass: false,
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

    fn wake(&self) -> FamilyWake {
        if self.cursor.is_some() {
            FamilyWake::Active
        } else if self.waiting_next_pass {
            FamilyWake::Waiting(PASS_INTERVAL_MS.saturating_sub(self.elapsed_ms))
        } else {
            FamilyWake::Idle
        }
    }
}

thread_local! {
    static DRIVERS: RefCell<[FamilyDriver; 3]> =
        RefCell::new([FamilyDriver::idle(), FamilyDriver::idle(), FamilyDriver::idle()]);
    static TOKENS: RefCell<HashMap<DeviceId, String>> = RefCell::new(HashMap::new());
    static ROUTES: RefCell<HashMap<FetchRequestId, InFlight>> = RefCell::new(HashMap::new());
}

const fn family_index(family: DeviceFamily) -> usize {
    match family {
        DeviceFamily::Bos => 0,
        DeviceFamily::Ubos => 1,
        DeviceFamily::Bitaxe => 2,
    }
}

fn with_driver<R>(family: DeviceFamily, f: impl FnOnce(&mut FamilyDriver) -> R) -> R {
    DRIVERS.with(|d| f(&mut d.borrow_mut()[family_index(family)]))
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

/// Discovery found a device; start a pass for any idle family. A family with no
/// devices stays idle (see `start_pass`), so this never parks an empty family.
pub fn ensure_running() {
    for family in FAMILIES {
        let should_start = with_driver(family, |d| d.cursor.is_none() && !d.waiting_next_pass);
        if should_start {
            start_pass(family);
        }
    }
    reschedule();
}

/// Accumulate frame time per family and start each family's next pass when due.
pub fn on_frame(delta_ms: u32) {
    for family in FAMILIES {
        let due = with_driver(family, |d| {
            if !d.waiting_next_pass {
                return false;
            }
            d.elapsed_ms = d.elapsed_ms.saturating_add(delta_ms);
            d.elapsed_ms >= PASS_INTERVAL_MS
        });
        if due {
            start_pass(family);
        }
    }
    reschedule();
}

/// Clear all cached tokens (e.g. after a password change).
pub fn clear_tokens() {
    TOKENS.with(|t| t.borrow_mut().clear());
}

/// Drop one device's cached session state and, if it is a family's current
/// device, abandon the in-flight pass for it so the cursor never stalls.
pub fn remove_token(id: &DeviceId) {
    TOKENS.with(|t| t.borrow_mut().remove(id));
    for family in FAMILIES {
        let is_current = with_driver(family, |d| d.current_device().as_ref() == Some(id));
        if is_current {
            abandon_current(family);
        }
    }
    reschedule();
}

/// Snapshot this family's devices and begin a pass. An empty snapshot leaves the
/// driver fully idle — it must not park as `waiting_next_pass`, or a device
/// discovered seconds later would not be polled until the 30s timer.
fn start_pass(family: DeviceFamily) {
    let ids = crate::DEVICES.with(|d| d.borrow().ids_for_family(family));
    if ids.is_empty() {
        with_driver(family, |d| {
            d.cursor = None;
            d.waiting_next_pass = false;
            d.elapsed_ms = 0;
        });
        return;
    }
    with_driver(family, |d| {
        d.elapsed_ms = 0;
        d.waiting_next_pass = false;
        d.cursor = Some(PassCursor::new(ids));
    });
    begin_device(family);
}

fn begin_device(family: DeviceFamily) {
    let done = with_driver(family, |d| {
        d.cursor.as_ref().is_none_or(PassCursor::is_done)
    });
    if done {
        with_driver(family, |d| {
            d.cursor = None;
            d.waiting_next_pass = true;
            d.elapsed_ms = 0;
        });
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
        issue_login(family, &id, &host, port, adapter);
    } else {
        fire_pending(family, &id, &host, port, adapter);
    }
}

/// Fire every still-pending (`None`) endpoint of the current device in parallel.
fn fire_pending(
    family: DeviceFamily,
    id: &DeviceId,
    host: &str,
    port: u16,
    adapter: &dyn FamilyAdapter,
) {
    let endpoints = adapter.telemetry_endpoints();
    let token = TOKENS.with(|t| t.borrow().get(id).cloned());
    let header = adapter
        .credential_header()
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
    let mut sent = 0_usize;
    for idx in pending_idxs {
        let url = fmt!("{}{}", base_url(adapter, host, port), endpoints[idx]);
        let req_id = FetchRequest::get(&url)
            .headers_opt(header.as_deref())
            .send(on_fetch);
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
) {
    let Some(auth_path) = adapter.auth_endpoint() else {
        finalize_failed(family, id);
        return;
    };
    let password = Params::current().miner_password;
    let url = fmt!("{}{}", base_url(adapter, host, port), auth_path);
    let body = adapter.login_body(&password);
    let generation = with_driver(family, |d| d.generation);
    let req_id = FetchRequest::post(&url)
        .headers("Content-Type: application/json")
        .body(body.as_bytes())
        .send(on_fetch);
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
    match route.kind {
        FetchKind::Login => on_login(route.family, &route.device, response),
        FetchKind::Telemetry { endpoint_idx } => {
            on_telemetry(route.family, endpoint_idx, response);
        }
    }
    reschedule();
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
        fire_pending(family, id, &host, port, adapter);
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
    let Some(id) = with_driver(family, |d| d.current_device()) else {
        return;
    };
    let Some((dev_family, _, _)) = resolve_identity(&id) else {
        return;
    };
    let Some(adapter) = adapter_for(dev_family) else {
        return;
    };
    let endpoints = adapter.telemetry_endpoints();
    let Some(ep) = endpoints.get(endpoint_idx) else {
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
            issue_login(family, &id, &host, port, adapter);
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
    crate::DEVICES.with(|devs| {
        let mut devs = devs.borrow_mut();
        devs.apply_telemetry(id, reading, reachable);
        if let Some(model) = model.into_model() {
            devs.apply_model(id, model);
        }
    });
    request_frame();
    advance_device(family);
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
    begin_device(family);
}

fn advance_device(family: DeviceFamily) {
    with_driver(family, |d| {
        if let Some(cursor) = d.cursor.as_mut() {
            cursor.advance();
        }
    });
    begin_device(family);
}

fn reschedule() {
    let wakes = DRIVERS.with(|d| {
        let d = d.borrow();
        [d[0].wake(), d[1].wake(), d[2].wake()]
    });
    if let Some(delay_ms) = next_wake(&wakes) {
        request_frame_after(delay_ms);
    }
}
