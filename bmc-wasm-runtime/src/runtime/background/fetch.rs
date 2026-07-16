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

//! Background HTTP fetch helpers for the WASM runtime.

use std::time::Duration;

use ureq::Agent;

pub(crate) fn build_fetch_agent() -> Agent {
    Agent::config_builder().build().into()
}

/// Perform an HTTP request, returning `(status_code, body)`.
/// Returns `(0, empty_body)` on network errors.
///
/// `timeout` is the per-call global cap on every ureq operation (DNS, connect,
/// send, recv). ureq 3.x defaults to no timeout, so without this a stalled peer
/// would hang the background fetch thread for OS-level TCP timeouts (minutes).
pub(in crate::runtime) fn do_fetch(
    agent: &Agent,
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
    timeout: Duration,
) -> (u32, Vec<u8>) {
    let result = match method {
        "POST" | "PUT" | "PATCH" => {
            let mut req = match method {
                "POST" => agent.post(url),
                "PUT" => agent.put(url),
                _ => agent.patch(url),
            }
            .config()
            .timeout_global(Some(timeout))
            .build();
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
            }
            .config()
            .timeout_global(Some(timeout))
            .build();
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

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    use super::{build_fetch_agent, do_fetch};

    #[test]
    fn per_call_timeout_trips_on_a_stalled_server() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("BUG: bind loopback");
        let addr = listener.local_addr().expect("BUG: local addr");
        let _stall = std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0_u8; 1024];
                let _ = sock.read(&mut buf);
                std::thread::sleep(Duration::from_secs(30));
            }
        });

        let agent = build_fetch_agent();
        let url = format!("http://{addr}/");
        let start = Instant::now();
        let (status, body) = do_fetch(&agent, "GET", &url, &[], None, Duration::from_millis(300));
        let elapsed = start.elapsed();

        assert_eq!(status, 0, "stalled fetch must surface as a network error");
        assert!(body.is_empty());
        assert!(
            elapsed < Duration::from_secs(5),
            "per-call timeout must trip before any OS-level timeout, took {elapsed:?}"
        );
    }
}
