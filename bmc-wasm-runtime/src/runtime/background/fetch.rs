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

use bmc_wasm_protocol::FetchOutcome;
use ureq::Agent;

/// Cap on a fetch response body — ureq's own default,
/// pinned so an upgrade cannot move it under us.
/// The value is arbitrary; nothing measured it.
const MAX_FETCH_BODY_BYTES: u64 = 10 * 1024 * 1024;

/// Cap on the response headers kept for a widget to read.
/// A few hundred bytes is normal; the cap bounds a hostile origin.
const MAX_FETCH_HEADER_BYTES: usize = 8 * 1_024;

pub(crate) fn build_fetch_agent() -> Agent {
    // A 4xx or 5xx is an answer, not a transport failure.
    // ureq would hand it back as an error carrying only the status,
    // dropping the headers and body the origin sent with it.
    Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

/// Whether a request may follow redirects.
#[derive(Clone, Copy)]
pub(in crate::runtime) enum Redirects {
    Follow,
    /// ureq strips only `authorization` across a redirect, so a secret
    /// in a custom header, the query or the body would be replayed
    /// to the target — and the guest picked the path.
    Refuse,
}

impl Redirects {
    pub(in crate::runtime) fn for_request(carries_secret: bool) -> Self {
        if carries_secret {
            Self::Refuse
        } else {
            Self::Follow
        }
    }
}

pub(in crate::runtime) struct FetchedReply {
    /// A [`FetchOutcome`] wire value.
    pub status: u32,
    /// Names arrive lowercased from `http`, so a lookup compares directly.
    pub headers: Vec<(String, String)>,
    /// For host-decided outcomes this carries a reason string instead,
    /// empty when the origin never answered.
    pub body: Vec<u8>,
}

/// The origin's headers, bounded by [`MAX_FETCH_HEADER_BYTES`].
/// A value that is not valid text is skipped: a mangled header
/// is worse to compare against than none.
fn kept_headers(headers: &ureq::http::HeaderMap) -> Vec<(String, String)> {
    let mut kept = Vec::new();
    let mut budget = MAX_FETCH_HEADER_BYTES;
    let mut skipped = 0_usize;
    for (name, value) in headers {
        let Ok(value) = value.to_str() else { continue };
        // Skipped, not stopped at: a fat header early would otherwise cost
        // every later one, and `Content-Type` has no fixed place.
        let Some(left) = budget.checked_sub(name.as_str().len() + value.len()) else {
            skipped += 1;
            continue;
        };
        budget = left;
        kept.push((name.as_str().to_owned(), value.to_owned()));
    }
    if skipped > 0 {
        tracing::warn!(
            skipped,
            cap = MAX_FETCH_HEADER_BYTES,
            "response headers dropped: the origin sent more than the cap"
        );
    }
    kept
}

/// Perform an HTTP request.
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
    redirects: Redirects,
) -> FetchedReply {
    // Zero returns the 3xx unfollowed rather than erroring,
    // so refusing costs a redirect the guest cannot use, not the request.
    let max_redirects = match redirects {
        Redirects::Follow => agent.config().max_redirects(),
        Redirects::Refuse => 0,
    };
    let result = match method {
        "POST" | "PUT" | "PATCH" => {
            let mut req = match method {
                "POST" => agent.post(url),
                "PUT" => agent.put(url),
                _ => agent.patch(url),
            }
            .config()
            .timeout_global(Some(timeout))
            .max_redirects(max_redirects)
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
            .max_redirects(max_redirects)
            .build();
            for (k, v) in headers {
                req = req.header(k, v);
            }
            req.call()
        }
    };
    let failed = |status: FetchOutcome, body: Vec<u8>| FetchedReply {
        status: status.to_wire(),
        headers: Vec::new(),
        body,
    };
    match result {
        Ok(response) => {
            let status = FetchOutcome::Http(response.status().as_u16()).to_wire();
            let headers = kept_headers(response.headers());
            let mut body = response.into_body();
            match body.with_config().limit(MAX_FETCH_BODY_BYTES).read_to_vec() {
                Ok(body) => FetchedReply {
                    status,
                    headers,
                    body,
                },
                Err(ureq::Error::BodyExceedsLimit(limit)) => failed(
                    FetchOutcome::BodyTooLarge,
                    format!("response body exceeds the {limit} byte limit").into_bytes(),
                ),
                Err(e) => failed(
                    FetchOutcome::Network,
                    format!("body read error: {e}").into_bytes(),
                ),
            }
        }
        // Unreachable while the agent keeps `http_status_as_error` off.
        // A status that did arrive must not read as a transport failure.
        Err(ureq::Error::StatusCode(code)) => failed(FetchOutcome::Http(code), Vec::new()),
        Err(_) => failed(FetchOutcome::Network, Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    use bmc_wasm_protocol::FetchOutcome;

    use super::{
        FetchedReply, MAX_FETCH_BODY_BYTES, MAX_FETCH_HEADER_BYTES, Redirects, build_fetch_agent,
        do_fetch,
    };

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
        let reply = do_fetch(
            &agent,
            "GET",
            &url,
            &[],
            None,
            Duration::from_millis(300),
            Redirects::Follow,
        );
        let elapsed = start.elapsed();

        assert_eq!(
            FetchOutcome::from_wire(reply.status),
            Some(FetchOutcome::Network),
            "stalled fetch must surface as a network error"
        );
        assert!(reply.body.is_empty());
        assert!(
            elapsed < Duration::from_secs(5),
            "per-call timeout must trip before any OS-level timeout, took {elapsed:?}"
        );
    }

    #[test]
    fn oversized_body_is_refused_as_its_own_outcome() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("BUG: bind loopback");
        let addr = listener.local_addr().expect("BUG: local addr");
        let oversized = MAX_FETCH_BODY_BYTES + 1;
        let _flood = std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0_u8; 1024];
                let _ = sock.read(&mut buf);
                let header =
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {oversized}\r\n\r\n").into_bytes();
                // The client hangs up once the cap trips; failing writes are the success path.
                if sock.write_all(&header).is_ok() {
                    let chunk = vec![0_u8; 64 * 1024];
                    let mut sent = 0_u64;
                    while sent < oversized && sock.write_all(&chunk).is_ok() {
                        sent += chunk.len() as u64;
                    }
                }
            }
        });

        let agent = build_fetch_agent();
        let url = format!("http://{addr}/");
        let reply = do_fetch(
            &agent,
            "GET",
            &url,
            &[],
            None,
            Duration::from_secs(30),
            Redirects::Follow,
        );

        assert_eq!(
            FetchOutcome::from_wire(reply.status),
            Some(FetchOutcome::BodyTooLarge),
            "an oversized body must not look like a network error"
        );
        let reason = String::from_utf8(reply.body).expect("BUG: reason string is UTF-8");
        assert!(
            reason.contains(&MAX_FETCH_BODY_BYTES.to_string()),
            "the reason must name the limit, got {reason:?}"
        );
    }

    #[test]
    fn the_content_type_survives_to_the_reply() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("BUG: bind loopback");
        let addr = listener.local_addr().expect("BUG: local addr");
        let _serve = std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0_u8; 1024];
                let _ = sock.read(&mut buf);
                let _ = sock.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}",
                );
            }
        });

        let agent = build_fetch_agent();
        let url = format!("http://{addr}/");
        let reply = do_fetch(
            &agent,
            "GET",
            &url,
            &[],
            None,
            Duration::from_secs(5),
            Redirects::Follow,
        );

        assert_eq!(
            FetchOutcome::from_wire(reply.status),
            Some(FetchOutcome::Http(200))
        );
        assert_eq!(
            header(&reply, "content-type"),
            Some("application/json"),
            "the origin's own header must reach the reply"
        );
        assert_eq!(reply.body, b"{}");
    }

    /// A refusal is an answer: its headers say why and for how long,
    /// which a widget can only act on if they survive the trip.
    #[test]
    fn an_error_status_keeps_the_headers_and_body_it_answered_with() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("BUG: bind loopback");
        let addr = listener.local_addr().expect("BUG: local addr");
        let _serve = std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0_u8; 1024];
                let _ = sock.read(&mut buf);
                // Each line ends in a literal `\r`, so the wire gets its CRLFs
                // while the source reads as the response it is.
                let _ = sock.write_all(
                    indoc::indoc! {"
                        HTTP/1.1 503 Service Unavailable\r
                        Content-Type: application/json\r
                        Retry-After: 120\r
                        Content-Length: 14\r
                        \r
                        {\"warming\":1}
                    "}
                    .as_bytes(),
                );
            }
        });

        let agent = build_fetch_agent();
        let url = format!("http://{addr}/");
        let reply = do_fetch(
            &agent,
            "GET",
            &url,
            &[],
            None,
            Duration::from_secs(5),
            Redirects::Follow,
        );

        assert_eq!(
            FetchOutcome::from_wire(reply.status),
            Some(FetchOutcome::Http(503))
        );
        assert_eq!(header(&reply, "retry-after"), Some("120"));
        assert_eq!(header(&reply, "content-type"), Some("application/json"));
        assert_eq!(
            reply.body, b"{\"warming\":1}\n",
            "an error's body is the origin's explanation, not something to drop"
        );
    }

    #[test]
    fn a_header_past_the_cap_does_not_cost_the_ones_after_it() {
        let filler = "x".repeat(1_000);
        let listener = TcpListener::bind("127.0.0.1:0").expect("BUG: bind loopback");
        let addr = listener.local_addr().expect("BUG: local addr");
        // `Content-Type` sits last, behind more filler than the cap allows:
        // exhausting the budget must not decide whether it arrives.
        let _serve = std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0_u8; 1024];
                let _ = sock.read(&mut buf);
                let mut reply = String::from("HTTP/1.1 200 OK\r\n");
                for i in 0..16 {
                    let _ = writeln!(reply, "X-Filler-{i}: {filler}\r");
                }
                reply.push_str("Content-Type: application/json\r\n");
                reply.push_str("Content-Length: 2\r\n\r\n{}");
                let _ = sock.write_all(reply.as_bytes());
            }
        });

        let agent = build_fetch_agent();
        let url = format!("http://{addr}/");
        let reply = do_fetch(
            &agent,
            "GET",
            &url,
            &[],
            None,
            Duration::from_secs(5),
            Redirects::Follow,
        );

        let kept: usize = reply
            .headers
            .iter()
            .map(|(name, value)| name.len() + value.len())
            .sum();
        assert!(
            kept <= MAX_FETCH_HEADER_BYTES,
            "kept {kept} bytes of headers, over the {MAX_FETCH_HEADER_BYTES} cap"
        );
        assert_eq!(
            header(&reply, "content-type"),
            Some("application/json"),
            "a header small enough to fit must land however late it arrives"
        );
    }

    fn header<'a>(reply: &'a FetchedReply, name: &str) -> Option<&'a str> {
        reply
            .headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}
