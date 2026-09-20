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

//! Following a Boser state stream: one task per sink holds the connection open,
//! decodes every snapshot into the sink's own state type,
//! and reconnects for as long as it lives.

use eventsource_stream::{EventStreamError, Eventsource};
use reqwest::Client;
use reqwest::redirect::Policy;
use serde::de::DeserializeOwned;
use serde_json::error::Category;
use std::convert::Infallible;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;
use tracing::{debug, info, warn};

/// What one stream's states mean to the rest of bmc.
pub trait StateSink: Send + 'static {
    type State: DeserializeOwned;

    /// Path under the Boser address, e.g. `/api/v1/upgrade/state/events`.
    const PATH: &'static str;

    /// Take one decoded state: the current one on every connection, then each change.
    fn observe(&mut self, state: &Self::State);

    /// React to valid JSON that this build's contract cannot decode.
    fn contract_mismatch(&mut self);

    /// React to an attempt that ended without a live stream, connected or not;
    /// the sink decides how much of its state survives.
    fn stream_lost(&mut self);
}

/// Where a Boser stream is, what authorizes it and how long each step may take.
#[derive(Debug, Clone)]
pub struct StreamConfig {
    pub address: SocketAddr,
    pub token_path: PathBuf,
    pub timing: Timing,
}

/// Deadlines within one connection and the pause between connections.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub response: Duration,
    pub first_event: Duration,
    pub idle: Duration,
    pub reconnect: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            response: Duration::from_secs(10),
            first_event: Duration::from_secs(10),
            // Four missed keep-alive comments at axum's default 15 s.
            idle: Duration::from_mins(1),
            // Boser is on localhost and supposed to be there: no backoff.
            reconnect: Duration::from_secs(5),
        }
    }
}

/// Run one task that follows the sink's stream, reconnecting until the handle is aborted.
pub fn spawn<S: StateSink>(config: StreamConfig, sink: S) -> JoinHandle<()> {
    tokio::spawn(run(config, sink))
}

async fn run<S: StateSink>(config: StreamConfig, mut sink: S) {
    let client = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .build()
        .expect("BUG: a static reqwest client configuration builds");
    let url = format!("http://{}{}", config.address, S::PATH);
    let mut health = Health::new(S::PATH);

    loop {
        let Err(error) = follow_stream(&client, &url, &config, &mut sink, &mut health).await;
        health.stream_lost(&error);
        sink.stream_lost();
        tokio::time::sleep(config.timing.reconnect).await;
    }
}

#[derive(Debug, Error)]
enum StreamError {
    #[error("token file unavailable: {0}")]
    TokenUnavailable(std::io::Error),
    #[error("token file is empty")]
    TokenEmpty,
    #[error("no response within {0:?}")]
    ResponseTimeout(Duration),
    #[error("request failed: {0}")]
    Request(reqwest::Error),
    #[error("response status {0}")]
    Status(reqwest::StatusCode),
    #[error("no state within {0:?} of the response")]
    FirstEventTimeout(Duration),
    #[error("stream idle for {0:?}")]
    Idle(Duration),
    #[error("stream ended")]
    Ended,
    #[error("stream transport failed: {0}")]
    Transport(reqwest::Error),
    #[error("malformed event stream: {0}")]
    Event(String),
}

impl StreamError {
    fn is_token_unreadable(&self) -> bool {
        matches!(
            self,
            Self::TokenUnavailable(error) if error.kind() != ErrorKind::NotFound
        )
    }
}

async fn follow_stream<S: StateSink>(
    client: &Client,
    url: &str,
    config: &StreamConfig,
    sink: &mut S,
    health: &mut Health,
) -> Result<Infallible, StreamError> {
    let timing = config.timing;
    let token = read_token(&config.token_path).await?;
    let request = client.get(url).bearer_auth(token).send();
    // Only the headers are bounded: the body is endless by design.
    let response = tokio::time::timeout(timing.response, request)
        .await
        .map_err(|_elapsed| StreamError::ResponseTimeout(timing.response))?
        .map_err(StreamError::Request)?;
    // Not `error_for_status`: it passes 3xx, and the client follows no redirect,
    // so a redirect's body is whatever answered in Boser's place.
    if !response.status().is_success() {
        return Err(StreamError::Status(response.status()));
    }
    // The idle deadline applies to bytes, so keep-alive comments reset it
    // although they never become events.
    let bytes = response
        .bytes_stream()
        .timeout(timing.idle)
        .map(move |item| match item {
            Ok(Ok(bytes)) => Ok(bytes),
            Ok(Err(error)) => Err(StreamError::Transport(error)),
            Err(_elapsed) => Err(StreamError::Idle(timing.idle)),
        });
    let mut events = std::pin::pin!(bytes.eventsource());

    // Boser replays the current state first, so a connection that delivers
    // no state promptly is broken even when comments keep it alive.
    let first_event_deadline = tokio::time::Instant::now() + timing.first_event;
    let mut received_state_event = false;
    loop {
        let next = if received_state_event {
            events.next().await
        } else {
            tokio::time::timeout_at(first_event_deadline, events.next())
                .await
                .map_err(|_elapsed| StreamError::FirstEventTimeout(timing.first_event))?
        };
        let Some(event) = next else {
            return Err(StreamError::Ended);
        };
        let event = event.map_err(|error| match error {
            EventStreamError::Transport(error) => error,
            EventStreamError::Utf8(error) => StreamError::Event(error.to_string()),
            EventStreamError::Parser(error) => StreamError::Event(error.to_string()),
        })?;
        if event.event != "message" {
            debug!(
                path = S::PATH,
                event = event.event,
                "ignoring named Boser event"
            );
            continue;
        }
        received_state_event = true;
        match serde_json::from_str::<S::State>(&event.data) {
            Ok(state) => {
                health.state_decoded();
                sink.observe(&state);
            }
            // A state newer than this build is replayed on every connection,
            // so reconnecting over it never recovers; data that is not JSON is corruption.
            Err(error) => match error.classify() {
                Category::Data => {
                    health.state_undecodable(&error, &event.data);
                    sink.contract_mismatch();
                }
                Category::Syntax | Category::Eof | Category::Io => {
                    return Err(StreamError::Event(error.to_string()));
                }
            },
        }
    }
}

async fn read_token(path: &Path) -> Result<String, StreamError> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .map_err(StreamError::TokenUnavailable)?;
    let token = contents.trim_end();
    if token.is_empty() {
        return Err(StreamError::TokenEmpty);
    }
    Ok(token.to_owned())
}

/// A stream that is expected to drop now and then, so one warning per outage;
/// every line names its stream, since several of them run at once.
#[derive(Debug)]
struct Health {
    path: &'static str,
    received_frame: bool,
    attempts_before_first_frame: usize,
    warned: bool,
    warned_undecodable: bool,
}

impl Health {
    /// Failed attempts a stream that never carried a frame keeps to itself:
    /// procd starts Boser (98) behind the compositor (95),
    /// so a cold boot opens with refusals no configuration can prevent,
    /// and a dozen of them at the 5 s reconnect is about a minute.
    const BOOT_ORDER_ATTEMPTS: usize = 12;

    fn new(path: &'static str) -> Self {
        Self {
            path,
            received_frame: false,
            attempts_before_first_frame: 0,
            warned: false,
            warned_undecodable: false,
        }
    }

    fn state_decoded(&mut self) {
        if self.warned {
            info!(path = self.path, "boser state stream recovered");
            self.warned = false;
        }
        self.received_frame = true;
        self.warned_undecodable = false;
    }

    /// Boser forwards every state change, download progress ticks included:
    /// one phase this build does not know would otherwise warn
    /// ten times a second for a whole download.
    fn state_undecodable(&mut self, error: &serde_json::Error, data: &str) {
        // The frame proves the connection but recovers nothing:
        // an outage ends on a state this build can act on, not on bytes.
        self.received_frame = true;
        if self.warned_undecodable {
            debug!(path = self.path, %error, data, "undecodable boser state");
        } else {
            warn!(path = self.path, %error, data, "undecodable boser state");
            self.warned_undecodable = true;
        }
    }

    fn stream_lost(&mut self, error: &StreamError) {
        if !self.received_frame {
            self.attempts_before_first_frame += 1;
        }
        // Before the first frame, a token Boser has not written yet and a
        // Boser still coming up behind the compositor are both boot order.
        // A token that is there but unreadable is neither, so no grace
        // hides it.
        let expected = !self.received_frame
            && self.attempts_before_first_frame <= Self::BOOT_ORDER_ATTEMPTS
            && !error.is_token_unreadable();
        if expected || self.warned {
            debug!(path = self.path, %error, "boser state stream unavailable");
        } else {
            warn!(path = self.path, %error, "boser state stream unavailable");
            self.warned = true;
        }
    }
}

#[cfg(test)]
mod tests;
