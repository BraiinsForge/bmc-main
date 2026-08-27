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

//! Unified fixture format for visual regression testing.
//!
//! A unified fixture is a single JSON file containing ALL events in a flat
//! timeline — user actions (clicks, captures), network events (WebSocket messages, mDNS
//! discoveries), and fetch responses — interleaved by `at_ms` timestamps. The fixture IS
//! the test: no separate hand-written interaction steps needed.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::system::SystemSnapshot;

// ── Header ──────────────────────────────────────────────────────────

/// Metadata stored at the top of a unified fixture file.
///
/// Captures the initial conditions so replay is fully self-contained:
/// start time, KV store state, and params snapshot are baked in, not
/// loaded from external files like `secrets.ini` or `manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureHeader {
    /// ISO 8601 start time (e.g. `"2026-03-10T18:00:00"`).
    pub time: String,
    /// Initial KV store entries. Applied before the first frame.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub kv: HashMap<String, String>,
    /// Initial params snapshot — the value of every manifest-declared key
    /// at the start of replay, before any `ParamDelivery` event fires. The
    /// runtime materialises this into `RuntimeConfig::params`, so the first
    /// `on_params_update` (driven by the first `ParamDelivery` in `events`)
    /// sees these values as `previous()`. JSON-shape matches `ParamDelivery`
    /// for round-trip simplicity.
    ///
    /// Empty for fixtures recorded before this field existed; capture
    /// replay falls back to an empty initial snapshot in that case, which
    /// means the first `ParamDelivery` will look like every key changed.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub initial_params: serde_json::Map<String, serde_json::Value>,
    /// Initial deck-wide system snapshot (timezone, formatting preferences,
    /// next-alarm) before any `SystemDelivery` event fires.
    ///
    /// Per-field `#[serde(default)]` on [`SystemSnapshot`] / [`crate::system::SystemSettings`]
    /// means fixtures recorded before this field existed (or before a given
    /// sub-field was added) fall through to typed defaults rather than
    /// failing fixture load.
    #[serde(default)]
    pub initial_system: SystemSnapshot,
    /// Which credential slots are bound at the start of replay,
    /// in the same JSON shape the `credentials` wayland event carries.
    ///
    /// The guest-visible half only — a fixture is a committed file,
    /// and nothing a widget renders depends on the secret values,
    /// so they have no reason to be here and cannot leak in.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub initial_credentials: serde_json::Map<String, serde_json::Value>,
}

// ── Body encoding ───────────────────────────────────────────────────

/// Binary-safe body encoding: JSON values stored natively, plain text as strings,
/// binary data as base64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureBody {
    /// Valid JSON — stored as a raw `serde_json::Value` so it appears unescaped
    /// in the fixture file (readable, diffable, grep-friendly).
    Json(serde_json::Value),
    /// Plain text (non-JSON UTF-8).
    Text(String),
    /// Binary data encoded as base64.
    #[serde(rename = "b64")]
    Base64(String),
}

impl FixtureBody {
    /// Create from raw bytes — tries JSON first, then plain text, then base64.
    #[must_use]
    pub fn from_bytes(data: &[u8]) -> Self {
        if let Ok(text) = std::str::from_utf8(data) {
            // Try parsing as JSON — only accept objects and arrays, not bare
            // strings/numbers which would lose their "text" semantics.
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(text)
                && (val.is_object() || val.is_array())
            {
                return Self::Json(val);
            }
            Self::Text(text.to_owned())
        } else {
            use base64::Engine;
            Self::Base64(base64::engine::general_purpose::STANDARD.encode(data))
        }
    }

    /// Decode to raw bytes.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Self::Json(val) => serde_json::to_string(val).unwrap_or_default().into_bytes(),
            Self::Text(s) => s.as_bytes().to_vec(),
            Self::Base64(b64) => {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .unwrap_or_default()
            }
        }
    }
}

// ── Event types ─────────────────────────────────────────────────────

/// A single event in the unified timeline.
///
/// Covers user interactions, frame captures, HTTP fetches, and all
/// network event types (SSDP, mDNS, WebSocket, TCP socket, UDP).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UnifiedEvent {
    // ── User actions ────────────────────────────────────────────
    /// Save screenshot frame(s).
    ///
    /// Without parameters: single frame. With `duration_ms` + `fps`:
    /// captures multiple frames over the given span (e.g. 2 s at 4 fps = 8 frames).
    Capture {
        /// Duration to capture over, in milliseconds. `None` = single frame.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        /// Frames per second during the capture span. Ignored without `duration_ms`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fps: Option<u32>,
    },
    /// Click the center of an element.
    Click {
        element: String,
    },
    /// Scroll an element by a pixel delta (negative = up).
    Scroll {
        element: String,
        delta: i32,
    },
    /// Drag across an element from one fractional X position to another.
    Drag {
        element: String,
        from: f32,
        to: f32,
    },
    /// Operator-driven params update — full snapshot delivered to
    /// the widget runtime. Replay calls `WasmWidgetRuntime::deliver_params_update`,
    /// which bumps the version counter and fires `on_params_update`.
    ///
    /// Capture writes one of these per change the operator made
    /// in the testbed param-mutation UI, plus one for the initial
    /// delivery so a fixture replay reproduces the pre-change state too.
    ///
    /// Values are stored as raw JSON for diffability; the `bmc-widget-manifest` parser
    /// re-derives the typed `ParamValue` at replay time, same as the wayland edge.
    ParamDelivery {
        params: serde_json::Map<String, serde_json::Value>,
    },
    /// Operator-driven system snapshot delivery.
    /// Replay calls `WasmWidgetRuntime::deliver_system_update`,
    /// which bumps the system version and fires `on_system_update`
    /// on the widget.
    SystemDelivery {
        system: SystemSnapshot,
    },
    /// Operator bound or unbound an account.
    /// Replay calls `WasmWidgetRuntime::deliver_credentials_update`,
    /// which fires `on_credentials_update` on the widget.
    ///
    /// Carries the guest-visible view only; replay pairs it
    /// with empty secrets, since no rendering can depend on them.
    CredentialDelivery {
        credentials: serde_json::Map<String, serde_json::Value>,
    },

    // ── HTTP fetch ──────────────────────────────────────────────
    /// A pre-recorded HTTP fetch response.
    Fetch {
        method: String,
        url: String,
        status: u32,
        body: FixtureBody,
    },

    // ── SSDP ────────────────────────────────────────────────────
    SsdpFound {
        search_id: u32,
        data: String,
    },
    SsdpRemoved {
        search_id: u32,
        data: String,
    },

    // ── mDNS ────────────────────────────────────────────────────
    MdnsFound {
        browse_id: u32,
        data: String,
    },
    MdnsRemoved {
        browse_id: u32,
        data: String,
    },

    // ── WebSocket ───────────────────────────────────────────────
    WsOpen {
        ws_id: u32,
    },
    WsMessage {
        ws_id: u32,
        data: FixtureBody,
    },
    WsClose {
        ws_id: u32,
        code: u16,
    },

    // ── TCP socket ──────────────────────────────────────────────
    SocketConnected {
        socket_id: u32,
    },
    SocketData {
        socket_id: u32,
        data: FixtureBody,
    },
    SocketClosed {
        socket_id: u32,
        code: u32,
    },

    // ── UDP broadcast ───────────────────────────────────────────
    UdpResponse {
        broadcast_id: u32,
        data: String,
        source: String,
    },

    // ── Audio ───────────────────────────────────────────────────
    /// An audio playback event (informational, no-op during replay).
    AudioPlay {
        /// Registry handle at event time; eviction may reuse it for another sample.
        sound_id: u32,
        volume: u32,
        /// Human-readable sample name (from registration).
        name: String,
        /// Sample duration in milliseconds.
        duration_ms: u32,
    },

    // ── LED ─────────────────────────────────────────────────────
    /// LED effect set, runs until superseded or stopped.
    LedSetEndless {
        effect: u8,
        r: u8,
        g: u8,
        b: u8,
        period_ms: u32,
        scope: u8,
    },
    /// LED effect set, runs for `duration_ms` ms then expires.
    LedSetTemporary {
        effect: u8,
        r: u8,
        g: u8,
        b: u8,
        period_ms: u32,
        duration_ms: u32,
        scope: u8,
    },
    /// All LED requests this widget owns are cancelled.
    LedStop,
}

/// A single timestamped event in the unified timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    /// Virtual time (monotonic ms) when this event fires.
    pub at_ms: u64,
    /// The event payload.
    #[serde(flatten)]
    pub event: UnifiedEvent,
}

// ── Top-level fixture ───────────────────────────────────────────────

/// A complete unified fixture: header metadata + flat event timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedFixture {
    /// Initial conditions (start time, KV state).
    pub header: FixtureHeader,
    /// Flat timeline of all events, ordered by `at_ms`.
    pub events: Vec<TimelineEvent>,
}

// ── Validation ──────────────────────────────────────────────────────

/// Validate a unified fixture for structural correctness.
///
/// Checks:
/// - `at_ms` values are monotonically non-decreasing
/// - `click` events have a non-empty `element` field
/// - At least one `capture` event exists
/// - Header has a non-empty `time` field
pub fn validate_fixture(fixture: &UnifiedFixture) -> Result<()> {
    if fixture.header.time.is_empty() {
        bail!("fixture header.time is empty");
    }

    let mut has_capture = false;
    let mut prev_ms: u64 = 0;

    for (i, event) in fixture.events.iter().enumerate() {
        // Monotonicity
        if event.at_ms < prev_ms {
            bail!(
                "events[{i}]: at_ms={} is less than previous at_ms={prev_ms} \
                 — timeline must be monotonically non-decreasing",
                event.at_ms
            );
        }
        prev_ms = event.at_ms;

        match &event.event {
            UnifiedEvent::Capture { .. } => has_capture = true,
            UnifiedEvent::Click { element } => {
                if element.is_empty() {
                    bail!("events[{i}]: click event has empty element ID");
                }
            }
            UnifiedEvent::Scroll { element, .. } => {
                if element.is_empty() {
                    bail!("events[{i}]: scroll event has empty element ID");
                }
            }
            UnifiedEvent::Drag { element, .. } => {
                if element.is_empty() {
                    bail!("events[{i}]: drag event has empty element ID");
                }
            }
            UnifiedEvent::Fetch { .. }
            | UnifiedEvent::ParamDelivery { .. }
            | UnifiedEvent::SystemDelivery { .. }
            | UnifiedEvent::CredentialDelivery { .. }
            | UnifiedEvent::SsdpFound { .. }
            | UnifiedEvent::SsdpRemoved { .. }
            | UnifiedEvent::MdnsFound { .. }
            | UnifiedEvent::MdnsRemoved { .. }
            | UnifiedEvent::WsOpen { .. }
            | UnifiedEvent::WsMessage { .. }
            | UnifiedEvent::WsClose { .. }
            | UnifiedEvent::SocketConnected { .. }
            | UnifiedEvent::SocketData { .. }
            | UnifiedEvent::SocketClosed { .. }
            | UnifiedEvent::UdpResponse { .. }
            | UnifiedEvent::AudioPlay { .. }
            | UnifiedEvent::LedSetEndless { .. }
            | UnifiedEvent::LedSetTemporary { .. }
            | UnifiedEvent::LedStop => {}
        }
    }

    if !has_capture {
        bail!("fixture has no capture events — at least one is required");
    }

    Ok(())
}

// ── Loading ─────────────────────────────────────────────────────────

/// Load a unified fixture from a JSONL gzip file (`.jsonl.gz`).
///
/// Line 1: `FixtureHeader`, remaining lines: `TimelineEvent` per line.
pub fn load_jsonl_fixture(path: &Path) -> Result<UnifiedFixture> {
    use std::io::BufRead;

    let file =
        std::fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = std::io::BufReader::new(flate2::read::GzDecoder::new(file));
    let mut lines = reader.lines();

    let header_line = lines
        .next()
        .context("fixture is empty — expected header on line 1")?
        .with_context(|| format!("failed to read header line from {}", path.display()))?;
    let header: FixtureHeader = serde_json::from_str(&header_line)
        .with_context(|| format!("failed to parse header in {}", path.display()))?;

    let mut events = Vec::new();
    for (i, line) in lines.enumerate() {
        let line =
            line.with_context(|| format!("failed to read line {} from {}", i + 2, path.display()))?;
        if line.is_empty() {
            continue;
        }
        let event: TimelineEvent = serde_json::from_str(&line).with_context(|| {
            format!(
                "failed to parse event on line {} in {}",
                i + 2,
                path.display()
            )
        })?;
        events.push(event);
    }

    Ok(UnifiedFixture { header, events })
}

/// Load a unified fixture from a `.jsonl.gz` file.
///
/// Delegates to [`load_jsonl_fixture`]. This is the single entry point
/// for fixture loading — only the JSONL format is supported.
pub fn load_unified_fixture(path: &Path) -> Result<UnifiedFixture> {
    load_jsonl_fixture(path)
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_fixture() -> UnifiedFixture {
        UnifiedFixture {
            header: FixtureHeader {
                time: "2026-03-10T18:00:00".into(),
                kv: HashMap::new(),
                initial_params: serde_json::Map::new(),
                initial_system: SystemSnapshot::default(),
                initial_credentials: serde_json::Map::new(),
            },
            events: vec![TimelineEvent {
                at_ms: 0,
                event: UnifiedEvent::Capture {
                    duration_ms: None,
                    fps: None,
                },
            }],
        }
    }

    // ── Validation ──────────────────────────────────────────────

    #[test]
    fn validate_minimal_fixture() {
        validate_fixture(&minimal_fixture()).expect("BUG: minimal fixture should be valid");
    }

    #[test]
    fn validate_rejects_empty_time() {
        let mut f = minimal_fixture();
        f.header.time = String::new();
        let err = validate_fixture(&f).expect_err("BUG: invalid fixture must fail validation");
        assert!(format!("{err:#}").contains("time is empty"));
    }

    #[test]
    fn validate_rejects_no_capture() {
        let f = UnifiedFixture {
            header: FixtureHeader {
                time: "2026-01-01T12:00:00".into(),
                kv: HashMap::new(),
                initial_params: serde_json::Map::new(),
                initial_system: SystemSnapshot::default(),
                initial_credentials: serde_json::Map::new(),
            },
            events: vec![TimelineEvent {
                at_ms: 0,
                event: UnifiedEvent::Click {
                    element: "btn".into(),
                },
            }],
        };
        let err = validate_fixture(&f).expect_err("BUG: invalid fixture must fail validation");
        assert!(format!("{err:#}").contains("no capture events"));
    }

    #[test]
    fn validate_rejects_non_monotonic() {
        let f = UnifiedFixture {
            header: FixtureHeader {
                time: "2026-01-01T12:00:00".into(),
                kv: HashMap::new(),
                initial_params: serde_json::Map::new(),
                initial_system: SystemSnapshot::default(),
                initial_credentials: serde_json::Map::new(),
            },
            events: vec![
                TimelineEvent {
                    at_ms: 100,
                    event: UnifiedEvent::Capture {
                        duration_ms: None,
                        fps: None,
                    },
                },
                TimelineEvent {
                    at_ms: 50,
                    event: UnifiedEvent::Capture {
                        duration_ms: None,
                        fps: None,
                    },
                },
            ],
        };
        let err = validate_fixture(&f).expect_err("BUG: invalid fixture must fail validation");
        assert!(format!("{err:#}").contains("monotonically"));
    }

    #[test]
    fn validate_rejects_empty_click_element() {
        let f = UnifiedFixture {
            header: FixtureHeader {
                time: "2026-01-01T12:00:00".into(),
                kv: HashMap::new(),
                initial_params: serde_json::Map::new(),
                initial_system: SystemSnapshot::default(),
                initial_credentials: serde_json::Map::new(),
            },
            events: vec![
                TimelineEvent {
                    at_ms: 0,
                    event: UnifiedEvent::Click {
                        element: String::new(),
                    },
                },
                TimelineEvent {
                    at_ms: 100,
                    event: UnifiedEvent::Capture {
                        duration_ms: None,
                        fps: None,
                    },
                },
            ],
        };
        let err = validate_fixture(&f).expect_err("BUG: invalid fixture must fail validation");
        assert!(format!("{err:#}").contains("empty element ID"));
    }

    #[test]
    fn validate_rejects_empty_scroll_element() {
        let f = UnifiedFixture {
            header: FixtureHeader {
                time: "2026-01-01T12:00:00".into(),
                kv: HashMap::new(),
                initial_params: serde_json::Map::new(),
                initial_system: SystemSnapshot::default(),
                initial_credentials: serde_json::Map::new(),
            },
            events: vec![
                TimelineEvent {
                    at_ms: 0,
                    event: UnifiedEvent::Scroll {
                        element: String::new(),
                        delta: -100,
                    },
                },
                TimelineEvent {
                    at_ms: 100,
                    event: UnifiedEvent::Capture {
                        duration_ms: None,
                        fps: None,
                    },
                },
            ],
        };
        let err = validate_fixture(&f).expect_err("BUG: invalid fixture must fail validation");
        assert!(format!("{err:#}").contains("empty element ID"));
    }

    #[test]
    fn validate_allows_equal_timestamps() {
        let f = UnifiedFixture {
            header: FixtureHeader {
                time: "2026-01-01T12:00:00".into(),
                kv: HashMap::new(),
                initial_params: serde_json::Map::new(),
                initial_system: SystemSnapshot::default(),
                initial_credentials: serde_json::Map::new(),
            },
            events: vec![
                TimelineEvent {
                    at_ms: 100,
                    event: UnifiedEvent::Click {
                        element: "btn".into(),
                    },
                },
                TimelineEvent {
                    at_ms: 100,
                    event: UnifiedEvent::Capture {
                        duration_ms: None,
                        fps: None,
                    },
                },
            ],
        };
        validate_fixture(&f).expect("BUG: equal timestamps should be allowed");
    }

    // ── Serialization round-trip ────────────────────────────────

    #[test]
    fn json_round_trip() {
        let fixture = UnifiedFixture {
            header: FixtureHeader {
                time: "2026-03-10T18:00:00".into(),
                kv: HashMap::from([("theme".into(), "dark".into())]),
                initial_params: serde_json::Map::new(),
                initial_system: SystemSnapshot::default(),
                initial_credentials: serde_json::Map::new(),
            },
            events: vec![
                TimelineEvent {
                    at_ms: 0,
                    event: UnifiedEvent::Capture {
                        duration_ms: None,
                        fps: None,
                    },
                },
                TimelineEvent {
                    at_ms: 50,
                    event: UnifiedEvent::Click {
                        element: "dev_0".into(),
                    },
                },
                TimelineEvent {
                    at_ms: 80,
                    event: UnifiedEvent::Fetch {
                        method: "GET".into(),
                        url: "https://example.com/api".into(),
                        status: 200,
                        body: FixtureBody::Text("{\"ok\":true}".into()),
                    },
                },
                TimelineEvent {
                    at_ms: 120,
                    event: UnifiedEvent::WsMessage {
                        ws_id: 1,
                        data: FixtureBody::Text("hello".into()),
                    },
                },
                TimelineEvent {
                    at_ms: 2_000,
                    event: UnifiedEvent::Capture {
                        duration_ms: None,
                        fps: None,
                    },
                },
            ],
        };

        let json = serde_json::to_string_pretty(&fixture).expect("BUG: serialize");
        let parsed: UnifiedFixture = serde_json::from_str(&json).expect("BUG: deserialize");

        assert_eq!(parsed.header.time, fixture.header.time);
        assert_eq!(parsed.header.kv, fixture.header.kv);
        assert_eq!(parsed.events.len(), fixture.events.len());
        for (a, b) in parsed.events.iter().zip(&fixture.events) {
            assert_eq!(a.at_ms, b.at_ms);
        }
    }

    #[test]
    fn json_captures_tag_format() {
        let event = TimelineEvent {
            at_ms: 0,
            event: UnifiedEvent::Capture {
                duration_ms: None,
                fps: None,
            },
        };
        let json = serde_json::to_string(&event).expect("BUG: serialize");
        assert!(json.contains(r#""type":"capture"#));
    }

    #[test]
    fn json_click_includes_element() {
        let event = TimelineEvent {
            at_ms: 50,
            event: UnifiedEvent::Click {
                element: "btn_0".into(),
            },
        };
        let json = serde_json::to_string(&event).expect("BUG: serialize");
        assert!(json.contains(r#""type":"click"#));
        assert!(json.contains(r#""element":"btn_0"#));
    }

    // ── FixtureBody ─────────────────────────────────────────────

    #[test]
    fn body_from_utf8_bytes() {
        let body = FixtureBody::from_bytes(b"hello world");
        assert_eq!(body, FixtureBody::Text("hello world".into()));
        assert_eq!(body.to_bytes(), b"hello world");
    }

    #[test]
    fn body_from_json_object() {
        let body = FixtureBody::from_bytes(br#"{"ok":true}"#);
        assert!(matches!(body, FixtureBody::Json(_)));
        // Round-trips back to compact JSON bytes
        assert_eq!(body.to_bytes(), br#"{"ok":true}"#);
    }

    #[test]
    fn body_from_json_array() {
        let body = FixtureBody::from_bytes(br"[1,2,3]");
        assert!(matches!(body, FixtureBody::Json(_)));
        assert_eq!(body.to_bytes(), br"[1,2,3]");
    }

    #[test]
    fn body_from_json_bare_string_stays_text() {
        // Bare JSON strings/numbers should remain Text, not Json
        let body = FixtureBody::from_bytes(br#""just a string""#);
        assert!(matches!(body, FixtureBody::Text(_)));
    }

    #[test]
    fn body_from_binary_bytes() {
        let data = vec![0xFF, 0x00, 0xAB];
        let body = FixtureBody::from_bytes(&data);
        assert!(matches!(body, FixtureBody::Base64(_)));
        assert_eq!(body.to_bytes(), data);
    }

    // ── All network event types serialize/deserialize ───────────

    #[test]
    fn all_network_event_types_round_trip() {
        let events = vec![
            UnifiedEvent::SsdpFound {
                search_id: 1,
                data: "ssdp-data".into(),
            },
            UnifiedEvent::SsdpRemoved {
                search_id: 1,
                data: "ssdp-data".into(),
            },
            UnifiedEvent::MdnsFound {
                browse_id: 2,
                data: "mdns-data".into(),
            },
            UnifiedEvent::MdnsRemoved {
                browse_id: 2,
                data: "mdns-data".into(),
            },
            UnifiedEvent::WsOpen { ws_id: 3 },
            UnifiedEvent::WsMessage {
                ws_id: 3,
                data: FixtureBody::Text("ws-msg".into()),
            },
            UnifiedEvent::WsClose {
                ws_id: 3,
                code: 1000,
            },
            UnifiedEvent::SocketConnected { socket_id: 4 },
            UnifiedEvent::SocketData {
                socket_id: 4,
                data: FixtureBody::Text("socket-data".into()),
            },
            UnifiedEvent::SocketClosed {
                socket_id: 4,
                code: 0,
            },
            UnifiedEvent::UdpResponse {
                broadcast_id: 5,
                data: "udp-data".into(),
                source: "127.0.0.1:1234".into(),
            },
        ];

        for event in events {
            let te = TimelineEvent { at_ms: 42, event };
            let json = serde_json::to_string(&te).expect("BUG: serialize");
            let parsed: TimelineEvent = serde_json::from_str(&json).expect("BUG: deserialize");
            assert_eq!(parsed.at_ms, 42);
        }
    }

    // ── JSONL round-trip ────────────────────────────────────────

    #[test]
    #[cfg(feature = "capture")]
    fn jsonl_gz_round_trip() {
        let fixture = UnifiedFixture {
            header: FixtureHeader {
                time: "2026-03-10T18:00:00".into(),
                kv: HashMap::from([("theme".into(), "dark".into())]),
                initial_params: serde_json::Map::new(),
                initial_system: SystemSnapshot::default(),
                initial_credentials: serde_json::Map::new(),
            },
            events: vec![
                TimelineEvent {
                    at_ms: 0,
                    event: UnifiedEvent::Capture {
                        duration_ms: None,
                        fps: None,
                    },
                },
                TimelineEvent {
                    at_ms: 50,
                    event: UnifiedEvent::Click {
                        element: "dev_0".into(),
                    },
                },
                TimelineEvent {
                    at_ms: 2_000,
                    event: UnifiedEvent::Capture {
                        duration_ms: Some(1_000),
                        fps: Some(4),
                    },
                },
            ],
        };

        let dir = tempfile::tempdir().expect("BUG: create tempdir");
        let path = dir.path().join("test.jsonl.gz");

        crate::fixtures::write_jsonl_fixture(&path, &fixture).expect("BUG: write jsonl");
        let loaded = load_jsonl_fixture(&path).expect("BUG: load jsonl");

        assert_eq!(loaded.header.time, fixture.header.time);
        assert_eq!(loaded.header.kv, fixture.header.kv);
        assert_eq!(loaded.events.len(), fixture.events.len());
        for (a, b) in loaded.events.iter().zip(&fixture.events) {
            assert_eq!(a.at_ms, b.at_ms);
        }
    }

    #[test]
    #[cfg(feature = "capture")]
    fn load_unified_dispatches_on_extension() {
        let fixture = minimal_fixture();
        let dir = tempfile::tempdir().expect("BUG: create tempdir");

        let jsonl_path = dir.path().join("test.jsonl.gz");
        crate::fixtures::write_jsonl_fixture(&jsonl_path, &fixture).expect("BUG: write jsonl");
        let loaded = load_unified_fixture(&jsonl_path).expect("BUG: load via dispatch");
        assert_eq!(loaded.header.time, fixture.header.time);
        assert_eq!(loaded.events.len(), fixture.events.len());
    }
}
