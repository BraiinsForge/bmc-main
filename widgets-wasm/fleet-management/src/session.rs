// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::device::DeviceId;

/// A one-pass cursor over a snapshot of device ids. Snapshotting at pass start
/// means devices added or removed mid-pass are only seen on the next pass.
pub struct PassCursor {
    ids: Vec<DeviceId>,
    index: usize,
}

impl PassCursor {
    #[must_use]
    pub fn new(ids: Vec<DeviceId>) -> Self {
        Self { ids, index: 0 }
    }

    #[must_use]
    pub fn current(&self) -> Option<&DeviceId> {
        self.ids.get(self.index)
    }

    pub fn advance(&mut self) {
        self.index += 1;
    }

    #[must_use]
    pub fn is_done(&self) -> bool {
        self.index >= self.ids.len()
    }
}

/// A device is reachable when its latest pass obtained usable telemetry from
/// at least one endpoint. Login failure leaves every endpoint failed, so this
/// also captures "shared-password rejected -> unreachable".
#[must_use]
pub fn pass_reachable(endpoint_oks: &[bool]) -> bool {
    endpoint_oks.iter().any(|&ok| ok)
}

#[cfg(target_arch = "wasm32")]
mod driver {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use bmc_wasm_sdk::ufmt;
    use bmc_wasm_sdk::{FetchRequest, fmt, log_warn, request_frame, request_frame_after};

    use super::{PassCursor, pass_reachable};
    use crate::adapter::FamilyAdapter;
    use crate::device::DeviceId;
    use crate::families::bos::BosAdapter;
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

    fn base_url(host: &str, port: u16) -> String {
        fmt!("http://{}:{}{}", host, port, BosAdapter.api_base_path())
    }

    /// Look up the host/port for the cursor's current device.
    fn current_endpoint() -> Option<(DeviceId, String, u16)> {
        let id = DRIVER.with(|d| d.borrow().cursor.as_ref()?.current().cloned())?;
        crate::DEVICES.with(|devs| {
            let devs = devs.borrow();
            let dev = devs.iter().find(|d| d.identity.id == id)?;
            Some((id.clone(), dev.identity.host.clone(), dev.identity.port))
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

    /// GET the current endpoint, attaching the auth header only if a token is
    /// already cached. No-auth families simply never have one cached.
    fn fetch_endpoint() {
        let endpoints = BosAdapter.telemetry_endpoints();
        let idx = DRIVER.with(|d| d.borrow().endpoint_idx);
        if idx >= endpoints.len() {
            finalize_device();
            return;
        }
        let Some((id, host, port)) = current_endpoint() else {
            advance_device();
            return;
        };
        let url = fmt!("{}{}", base_url(&host, port), endpoints[idx]);
        let token = TOKENS.with(|t| t.borrow().get(&id).cloned());
        let header = token.map(|t| BosAdapter.auth_header(&t));
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
        let endpoints = BosAdapter.telemetry_endpoints();
        let idx = DRIVER.with(|d| d.borrow().endpoint_idx);
        let Some(ep) = endpoints.get(idx) else {
            advance_device();
            return;
        };

        let can_reauth = BosAdapter.auth_endpoint().is_some()
            && BosAdapter.is_auth_error(response.status)
            && !DRIVER.with(|d| d.borrow().reauthed);
        if can_reauth {
            DRIVER.with(|d| d.borrow_mut().reauthed = true);
            if let Some((id, _, _)) = current_endpoint() {
                TOKENS.with(|t| t.borrow_mut().remove(&id));
            }
            login_then_retry();
            return;
        }

        if response.ok() {
            let doc = response.json();
            DRIVER.with(|d| {
                let mut d = d.borrow_mut();
                let d = &mut *d;
                BosAdapter.parse_telemetry(ep, &doc, &mut d.reading);
                BosAdapter.parse_model(ep, &doc, &mut d.model);
            });
            record_endpoint(true);
        } else {
            DRIVER.with(|d| BosAdapter.reset_telemetry(ep, &mut d.borrow_mut().reading));
            record_endpoint(false);
        }
    }

    /// Log in with the shared password, then retry the SAME endpoint with the
    /// fresh token. A failed login leaves the endpoint as N/A and advances.
    fn login_then_retry() {
        let Some((_, host, port)) = current_endpoint() else {
            advance_device();
            return;
        };
        let Some(auth_path) = BosAdapter.auth_endpoint() else {
            record_endpoint(false);
            return;
        };
        let password = Params::current().miner_password;
        let url = fmt!("{}{}", base_url(&host, port), auth_path);
        let body = BosAdapter.login_body(&password);
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
        let token = if response.ok() {
            BosAdapter.parse_login(&response.json())
        } else {
            None
        };
        if let (Some((id, _, _)), Some(token)) = (current_endpoint(), token) {
            TOKENS.with(|t| t.borrow_mut().insert(id, token));
            // Retry the same endpoint (endpoint_idx unchanged) with the token.
            fetch_endpoint();
        } else {
            // Login failed: this endpoint is N/A; move on.
            let endpoints = BosAdapter.telemetry_endpoints();
            let idx = DRIVER.with(|d| d.borrow().endpoint_idx);
            if let Some(ep) = endpoints.get(idx) {
                DRIVER.with(|d| BosAdapter.reset_telemetry(ep, &mut d.borrow_mut().reading));
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
}

#[cfg(target_arch = "wasm32")]
pub use driver::{clear_tokens, ensure_running, on_frame, remove_token};

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<DeviceId> {
        (0..n)
            .map(|i| DeviceId::new(format!("d{i}._http._tcp.local.")))
            .collect()
    }

    #[test]
    fn cursor_advances_then_completes() {
        let mut c = PassCursor::new(ids(2));
        assert_eq!(
            c.current().map(DeviceId::as_str),
            Some("d0._http._tcp.local.")
        );
        assert!(!c.is_done());
        c.advance();
        assert_eq!(
            c.current().map(DeviceId::as_str),
            Some("d1._http._tcp.local.")
        );
        c.advance();
        assert!(c.is_done());
        assert_eq!(c.current(), None);
    }

    #[test]
    fn empty_cursor_is_immediately_done() {
        let c = PassCursor::new(ids(0));
        assert!(c.is_done());
        assert_eq!(c.current(), None);
    }

    #[test]
    fn cursor_iterates_exactly_its_snapshot_in_order() {
        let mut c = PassCursor::new(ids(3));
        let mut seen = Vec::new();
        while let Some(id) = c.current() {
            seen.push(id.as_str().to_owned());
            c.advance();
        }
        assert_eq!(
            seen,
            vec![
                "d0._http._tcp.local.".to_owned(),
                "d1._http._tcp.local.".to_owned(),
                "d2._http._tcp.local.".to_owned(),
            ]
        );
        assert!(c.is_done());
    }

    #[test]
    fn reachable_only_when_an_endpoint_succeeded() {
        assert!(!pass_reachable(&[]));
        assert!(!pass_reachable(&[false, false]));
        assert!(pass_reachable(&[false, true, false]));
    }
}
