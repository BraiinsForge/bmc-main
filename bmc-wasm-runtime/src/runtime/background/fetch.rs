// Copyright (C) 2026  Braiins Systems s.r.o.

//! Background HTTP fetch helpers for the WASM runtime.

use std::time::Duration;

use ureq::Agent;

/// Global cap on every ureq operation (DNS, connect, send, recv).
/// ureq 3.x defaults to no timeout, so without this a stalled peer can
/// hang the background fetch thread for OS-level TCP timeouts (minutes).
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn build_fetch_agent() -> Agent {
    Agent::config_builder()
        .timeout_global(Some(FETCH_TIMEOUT))
        .build()
        .into()
}

/// Perform an HTTP request, returning `(status_code, body)`.
/// Returns `(0, empty_body)` on network errors.
pub(in crate::runtime) fn do_fetch(
    agent: &Agent,
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> (u32, Vec<u8>) {
    let result = match method {
        "POST" | "PUT" | "PATCH" => {
            let mut req = match method {
                "POST" => agent.post(url),
                "PUT" => agent.put(url),
                _ => agent.patch(url),
            };
            for (k, v) in headers {
                req = req.header(k, v);
            }
            match body {
                Some(bytes) => req.send(bytes),
                None => req.send_empty(),
            }
        }
        _ => {
            let mut req = match method {
                "DELETE" => agent.delete(url),
                "HEAD" => agent.head(url),
                _ => agent.get(url),
            };
            for (k, v) in headers {
                req = req.header(k, v);
            }
            req.call()
        }
    };
    match result {
        Ok(response) => {
            let status = u32::from(response.status().as_u16());
            match response.into_body().read_to_vec() {
                Ok(body) => (status, body),
                Err(e) => (0, format!("body read error: {e}").into_bytes()),
            }
        }
        Err(ureq::Error::StatusCode(code)) => (u32::from(code), Vec::new()),
        Err(_) => (0, Vec::new()),
    }
}
