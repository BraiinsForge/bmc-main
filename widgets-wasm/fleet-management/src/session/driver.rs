// Copyright (C) 2026  Braiins Systems s.r.o.

use std::cell::RefCell;
use std::collections::HashMap;

use bmc_wasm_sdk::ufmt;
use bmc_wasm_sdk::{FetchRequest, fmt, log_warn, request_frame, request_frame_after};

use super::{PassCursor, adapter_for, pass_reachable};
use crate::adapter::FamilyAdapter;
use crate::device::{DeviceFamily, DeviceId};
use crate::manifest_params::Params;
use crate::model::ModelAccumulator;
use crate::telemetry::TelemetryReading;

const PASS_INTERVAL_MS: u32 = 30_000;

struct Driver {
    cursor: Option<PassCursor>,
    endpoint_idx: usize,
    reading: TelemetryReading,
    model: ModelAccumulator,
    endpoint_oks: Vec<bool>,
    // One re-auth attempt per device per pass guards against a 401 -> login
    // -> 401 loop when the shared password is wrong.
    reauthed: bool,
    elapsed_ms: u32,
    waiting_next_pass: bool,
}

impl Driver {
    const fn idle() -> Self {
        Self {
            cursor: None,
            endpoint_idx: 0,
            reading: TelemetryReading {
                current_hashrate_ths: None,
                nominal_hashrate_ths: None,
                power_w: None,
                uptime_s: None,
                temperature_c: None,
            },
            model: ModelAccumulator {
                id: None,
                name: None,
                chip_type: None,
                chip_count: None,
            },
            endpoint_oks: Vec::new(),
            reauthed: false,
            elapsed_ms: 0,
            waiting_next_pass: false,
        }
    }
}

thread_local! {
    static DRIVER: RefCell<Driver> = const { RefCell::new(Driver::idle()) };
    static TOKENS: RefCell<HashMap<DeviceId, String>> = RefCell::new(HashMap::new());
}

fn base_url(adapter: &dyn FamilyAdapter, host: &str, port: u16) -> String {
    fmt!("http://{}:{}{}", host, port, adapter.api_base_path())
}

/// Look up the family, host, and port for the cursor's current device.
fn current_endpoint() -> Option<(DeviceId, DeviceFamily, String, u16)> {
    let id = DRIVER.with(|d| d.borrow().cursor.as_ref()?.current().cloned())?;
    crate::DEVICES.with(|devs| {
        let devs = devs.borrow();
        let dev = devs.iter().find(|d| d.identity.id == id)?;
        Some((
            id.clone(),
            dev.identity.family,
            dev.identity.host.clone(),
            dev.identity.port,
        ))
    })
}

/// Discovery found a device; start a pass if the driver is idle.
pub fn ensure_running() {
    let should_start = DRIVER.with(|d| {
        let d = d.borrow();
        d.cursor.is_none() && !d.waiting_next_pass
    });
    if should_start {
        start_pass();
    }
}

/// Accumulate frame time and start the next pass once the interval elapses.
pub fn on_frame(delta_ms: u32) {
    let start = DRIVER.with(|d| {
        let mut d = d.borrow_mut();
        d.elapsed_ms = d.elapsed_ms.saturating_add(delta_ms);
        d.waiting_next_pass && d.elapsed_ms >= PASS_INTERVAL_MS
    });
    if start {
        start_pass();
    }
}

/// Clear all cached tokens (e.g. after a password change).
pub fn clear_tokens() {
    TOKENS.with(|t| t.borrow_mut().clear());
}

/// Drop one device's cached session state when discovery removes it.
pub fn remove_token(id: &DeviceId) {
    TOKENS.with(|t| t.borrow_mut().remove(id));
}

fn start_pass() {
    let ids = crate::DEVICES.with(|d| d.borrow().ids());
    DRIVER.with(|d| {
        let mut d = d.borrow_mut();
        d.elapsed_ms = 0;
        d.waiting_next_pass = false;
        if ids.is_empty() {
            d.cursor = None;
        } else {
            d.cursor = Some(PassCursor::new(ids));
        }
    });
    // Wake near the 30s mark to pace the next pass even while idle.
    request_frame_after(PASS_INTERVAL_MS);
    begin_device();
}

fn begin_device() {
    let done = DRIVER.with(|d| d.borrow().cursor.as_ref().is_none_or(PassCursor::is_done));
    if done {
        // Re-arm the wake explicitly: request_frame() calls during the pass
        // may have superseded the pass-start timer, so guarantee a frame at
        // the remaining time to the 30s mark rather than relying on it.
        let remaining = DRIVER.with(|d| {
            let mut d = d.borrow_mut();
            d.cursor = None;
            d.waiting_next_pass = true;
            PASS_INTERVAL_MS.saturating_sub(d.elapsed_ms)
        });
        request_frame_after(remaining.max(1));
        return;
    }
    // A device whose family has no adapter yet cannot be polled. `upsert`
    // left it reachable, so mark it unreachable with cleared telemetry and
    // move on, rather than assuming the family is never discovered.
    if let Some((id, family, _, _)) = current_endpoint()
        && adapter_for(family).is_none()
    {
        log_warn!("fleet: no adapter for discovered device family; marking unreachable");
        crate::DEVICES.with(|devs| {
            devs.borrow_mut()
                .apply_telemetry(&id, TelemetryReading::default(), false);
        });
        request_frame();
        advance_device();
        return;
    }
    DRIVER.with(|d| {
        let mut d = d.borrow_mut();
        d.endpoint_idx = 0;
        d.endpoint_oks.clear();
        d.reauthed = false;
        d.reading = Driver::idle().reading;
        d.model = ModelAccumulator::default();
    });
    fetch_endpoint();
}

/// GET the current endpoint via its family adapter, attaching the adapter's
/// proactive credential header if any, else a cached token if present.
fn fetch_endpoint() {
    let Some((id, family, host, port)) = current_endpoint() else {
        advance_device();
        return;
    };
    let Some(adapter) = adapter_for(family) else {
        advance_device();
        return;
    };
    let endpoints = adapter.telemetry_endpoints();
    let idx = DRIVER.with(|d| d.borrow().endpoint_idx);
    if idx >= endpoints.len() {
        finalize_device();
        return;
    }
    let url = fmt!("{}{}", base_url(adapter, &host, port), endpoints[idx]);
    let token = TOKENS.with(|t| t.borrow().get(&id).cloned());
    let header = adapter
        .credential_header()
        .or_else(|| token.map(|t| adapter.auth_header(&t)));
    if FetchRequest::get(&url)
        .headers_opt(header.as_deref())
        .send(on_endpoint)
        .is_none()
    {
        log_warn!("fleet: telemetry send rejected for {}", host);
        record_endpoint(false);
    }
}

/// React to a telemetry reply: re-authenticate on an adapter-reported auth
/// error (once per device per pass), else fold or reset the endpoint.
fn on_endpoint(response: &bmc_wasm_sdk::FetchResponse) {
    let Some((id, family, _, _)) = current_endpoint() else {
        advance_device();
        return;
    };
    let Some(adapter) = adapter_for(family) else {
        advance_device();
        return;
    };
    let endpoints = adapter.telemetry_endpoints();
    let idx = DRIVER.with(|d| d.borrow().endpoint_idx);
    let Some(ep) = endpoints.get(idx) else {
        advance_device();
        return;
    };

    let can_reauth = adapter.auth_endpoint().is_some()
        && adapter.is_auth_error(response.status)
        && !DRIVER.with(|d| d.borrow().reauthed);
    if can_reauth {
        DRIVER.with(|d| d.borrow_mut().reauthed = true);
        TOKENS.with(|t| t.borrow_mut().remove(&id));
        login_then_retry();
        return;
    }

    if response.ok() {
        let doc = response.json();
        DRIVER.with(|d| {
            let mut d = d.borrow_mut();
            let d = &mut *d;
            adapter.parse_telemetry(ep, &doc, &mut d.reading);
            adapter.parse_model(ep, &doc, &mut d.model);
        });
        record_endpoint(true);
    } else {
        DRIVER.with(|d| adapter.reset_telemetry(ep, &mut d.borrow_mut().reading));
        record_endpoint(false);
    }
}

/// Log in with the shared password, then retry the SAME endpoint with the
/// fresh token. A failed login leaves the endpoint as N/A and advances.
fn login_then_retry() {
    let Some((_, family, host, port)) = current_endpoint() else {
        advance_device();
        return;
    };
    let Some(adapter) = adapter_for(family) else {
        advance_device();
        return;
    };
    let Some(auth_path) = adapter.auth_endpoint() else {
        record_endpoint(false);
        return;
    };
    let password = Params::current().miner_password;
    let url = fmt!("{}{}", base_url(adapter, &host, port), auth_path);
    let body = adapter.login_body(&password);
    if FetchRequest::post(&url)
        .headers("Content-Type: application/json")
        .body(body.as_bytes())
        .send(on_login)
        .is_none()
    {
        log_warn!("fleet: login send rejected for {}", host);
        record_endpoint(false);
    }
}

fn on_login(response: &bmc_wasm_sdk::FetchResponse) {
    let Some((id, family, _, _)) = current_endpoint() else {
        advance_device();
        return;
    };
    let Some(adapter) = adapter_for(family) else {
        advance_device();
        return;
    };
    let token = if response.ok() {
        adapter.parse_login(&response.json())
    } else {
        None
    };
    if let Some(token) = token {
        TOKENS.with(|t| t.borrow_mut().insert(id, token));
        // Retry the same endpoint (endpoint_idx unchanged) with the token.
        fetch_endpoint();
    } else {
        // Login failed: this endpoint is N/A; move on.
        let endpoints = adapter.telemetry_endpoints();
        let idx = DRIVER.with(|d| d.borrow().endpoint_idx);
        if let Some(ep) = endpoints.get(idx) {
            DRIVER.with(|d| adapter.reset_telemetry(ep, &mut d.borrow_mut().reading));
        }
        record_endpoint(false);
    }
}

/// Record the current endpoint's outcome and move to the next one.
fn record_endpoint(ok: bool) {
    DRIVER.with(|d| {
        let mut d = d.borrow_mut();
        d.endpoint_oks.push(ok);
        d.endpoint_idx += 1;
    });
    fetch_endpoint();
}

fn finalize_device() {
    let id = DRIVER.with(|d| {
        d.borrow()
            .cursor
            .as_ref()
            .and_then(|c| c.current().cloned())
    });
    if let Some(id) = id {
        let (reading, reachable, model) = DRIVER.with(|d| {
            let d = d.borrow();
            (d.reading, pass_reachable(&d.endpoint_oks), d.model.clone())
        });
        crate::DEVICES.with(|devs| {
            let mut devs = devs.borrow_mut();
            devs.apply_telemetry(&id, reading, reachable);
            if let Some(model) = model.into_model() {
                devs.apply_model(&id, model);
            }
        });
        request_frame();
    }
    advance_device();
}

fn advance_device() {
    DRIVER.with(|d| {
        if let Some(cursor) = d.borrow_mut().cursor.as_mut() {
            cursor.advance();
        }
    });
    begin_device();
}
