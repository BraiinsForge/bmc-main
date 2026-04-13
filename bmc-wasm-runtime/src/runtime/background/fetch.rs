// Copyright (C) 2026  Braiins Systems s.r.o.

//! Background HTTP fetch helpers for the WASM runtime.

/// Perform an HTTP request, returning `(status_code, body)`.
/// Returns `(0, empty_body)` on network errors.
pub(in crate::runtime) fn do_fetch(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> (u32, Vec<u8>) {
    let result = match method {
        "POST" | "PUT" | "PATCH" => {
            let mut req = match method {
                "POST" => ureq::post(url),
                "PUT" => ureq::put(url),
                _ => ureq::patch(url),
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
                "DELETE" => ureq::delete(url),
                "HEAD" => ureq::head(url),
                _ => ureq::get(url),
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
