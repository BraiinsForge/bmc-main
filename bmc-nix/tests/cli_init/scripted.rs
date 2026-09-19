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

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use super::*;

pub(crate) enum Reply {
    Http {
        status: u16,
        declared_len: Option<usize>,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    Drop,
}

impl Reply {
    pub(crate) fn full(body: &[u8]) -> Self {
        Self::Http {
            status: 200,
            declared_len: Some(body.len()),
            headers: Vec::new(),
            body: body.to_vec(),
        }
    }

    pub(crate) fn short(body: &[u8], declared_len: usize) -> Self {
        Self::Http {
            status: 200,
            declared_len: Some(declared_len),
            headers: Vec::new(),
            body: body.to_vec(),
        }
    }

    pub(crate) fn range(body: &[u8], start: usize, total: usize) -> Self {
        Self::Http {
            status: 206,
            declared_len: Some(body.len()),
            headers: vec![(
                "Content-Range".to_owned(),
                format!("bytes {start}-{}/{total}", start + body.len() - 1),
            )],
            body: body.to_vec(),
        }
    }

    pub(crate) fn short_range(body: &[u8], start: usize, total: usize) -> Self {
        Self::Http {
            status: 206,
            declared_len: Some(total - start),
            headers: vec![(
                "Content-Range".to_owned(),
                format!("bytes {start}-{}/{total}", total - 1),
            )],
            body: body.to_vec(),
        }
    }

    pub(crate) fn status(status: u16) -> Self {
        Self::Http {
            status,
            declared_len: Some(0),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// A 200 delimited by the connection closing, as HTTP/1.x still allows.
    pub(crate) fn close_delimited(body: &[u8]) -> Self {
        Self::Http {
            status: 200,
            declared_len: None,
            headers: Vec::new(),
            body: body.to_vec(),
        }
    }

    pub(crate) fn with_header(mut self, name: &str, value: &str) -> Self {
        if let Self::Http { headers, .. } = &mut self {
            headers.push((name.to_owned(), value.to_owned()));
        }
        self
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TarballRequest {
    pub(crate) range: Option<String>,
    pub(crate) if_range: Option<String>,
}

pub(crate) struct ScriptedServer {
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    feed_hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<TarballRequest>>>,
    handle: Option<JoinHandle<()>>,
}

impl ScriptedServer {
    pub(crate) fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub(crate) fn feed_hits(&self) -> usize {
        self.feed_hits.load(Ordering::SeqCst)
    }

    pub(crate) fn requests(&self) -> Vec<TarballRequest> {
        self.requests
            .lock()
            .expect("BUG: requests lock poisoned")
            .clone()
    }

    pub(crate) fn ranges(&self) -> Vec<Option<String>> {
        self.requests()
            .into_iter()
            .map(|request| request.range)
            .collect()
    }

    pub(crate) fn if_ranges(&self) -> Vec<Option<String>> {
        self.requests()
            .into_iter()
            .map(|request| request.if_range)
            .collect()
    }
}

impl Drop for ScriptedServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(handle) = self.handle.take() {
            handle.join().expect("BUG: scripted HTTP server panicked");
        }
    }
}

pub(crate) fn serve_scripted(
    listener: TcpListener,
    addr: SocketAddr,
    feed: Vec<u8>,
    feed_failures: Vec<Reply>,
    replies: Vec<Reply>,
) -> ScriptedServer {
    let shutdown = Arc::new(AtomicBool::new(false));
    let feed_hits = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let thread_shutdown = Arc::clone(&shutdown);
    let thread_feed_hits = Arc::clone(&feed_hits);
    let thread_requests = Arc::clone(&requests);
    let handle = std::thread::spawn(move || {
        let mut replies: VecDeque<_> = replies.into();
        let mut feed_failures: VecDeque<_> = feed_failures.into();
        for stream in listener.incoming() {
            if thread_shutdown.load(Ordering::SeqCst) {
                break;
            }
            let stream = stream.expect("BUG: accept test connection");
            let mut reader =
                BufReader::new(stream.try_clone().expect("BUG: clone test connection"));
            let mut request = String::new();
            if reader.read_line(&mut request).expect("BUG: read request") == 0 {
                continue;
            }
            let path = request
                .split_whitespace()
                .nth(1)
                .expect("BUG: request has a path");
            let mut tarball_request = TarballRequest::default();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).expect("BUG: read header") == 0 || line == "\r\n" {
                    break;
                }
                let (name, value) = line.split_once(':').unwrap_or((&line, ""));
                let value = Some(value.trim().to_owned());
                if name.eq_ignore_ascii_case("range") {
                    tarball_request.range = value;
                } else if name.eq_ignore_ascii_case("if-range") {
                    tarball_request.if_range = value;
                }
            }
            let mut stream = reader.into_inner();
            if path == "/nix-package-feed.v1.json" {
                thread_feed_hits.fetch_add(1, Ordering::SeqCst);
                send_reply(
                    &mut stream,
                    feed_failures
                        .pop_front()
                        .unwrap_or_else(|| Reply::full(&feed)),
                );
                continue;
            }
            thread_requests
                .lock()
                .expect("BUG: requests lock poisoned")
                .push(tarball_request);
            send_reply(
                &mut stream,
                replies
                    .pop_front()
                    .expect("BUG: unexpected tarball request"),
            );
        }
    });
    ScriptedServer {
        addr,
        shutdown,
        feed_hits,
        requests,
        handle: Some(handle),
    }
}

fn send_reply(stream: &mut TcpStream, reply: Reply) {
    let Reply::Http {
        status,
        declared_len,
        headers,
        body,
    } = reply
    else {
        return;
    };
    let mut header = format!("HTTP/1.1 {status} Test\r\nConnection: close\r\n");
    if let Some(declared_len) = declared_len {
        write!(header, "Content-Length: {declared_len}\r\n")
            .expect("BUG: formatting into a String cannot fail");
    }
    for (name, value) in headers {
        header.push_str(&name);
        header.push_str(": ");
        header.push_str(&value);
        header.push_str("\r\n");
    }
    header.push_str("\r\n");
    stream
        .write_all(header.as_bytes())
        .expect("BUG: write test headers");
    stream.write_all(&body).expect("BUG: write test body");
}
