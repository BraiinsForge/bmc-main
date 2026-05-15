// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared fixture utilities for capture and testbed binaries.
//!
//! Provides unified fixture recording, writing, and KV seeding for
//! headless capture and interactive testbed sessions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::{FixtureEvent, FixtureEventKind, RuntimeConfig};

// ── Unified recording config ─────────────────────────────────────────

/// Build a [`RuntimeConfig`] for unified fixture recording mode.
///
/// Instead of writing fetches to `fetch_responses.json`, the observer appends
/// `TimelineEvent::Fetch` entries to a shared vec with `at_ms` computed from
/// the provided `start_instant`. Network events are recorded via the runtime's
/// `record_events` flag and retrieved with `take_recorded_events()`.
#[must_use]
#[expect(clippy::cast_possible_truncation)]
pub fn build_unified_recording_config(
    kv_dir: PathBuf,
    fetch_events: std::sync::Arc<std::sync::Mutex<Vec<crate::unified_fixture::TimelineEvent>>>,
    start_instant: std::time::Instant,
) -> RuntimeConfig {
    use crate::unified_fixture::{FixtureBody, TimelineEvent, UnifiedEvent};

    let mut rt_config = RuntimeConfig {
        kv_store_path: Some(kv_dir),
        record_events: true,
        rng_seed: Some(42),
        ..RuntimeConfig::default()
    };

    let events = fetch_events;
    rt_config.fetch_observer = Some(Box::new(move |key, status, body| {
        // Parse "METHOD URL" key
        let (method, url) = key.split_once(' ').unwrap_or(("GET", key));
        let at_ms = start_instant.elapsed().as_millis() as u64;
        let event = TimelineEvent {
            at_ms,
            event: UnifiedEvent::Fetch {
                method: method.to_owned(),
                url: url.to_owned(),
                status,
                body: FixtureBody::from_bytes(body),
            },
        };
        events
            .lock()
            .expect("BUG: fetch events poisoned")
            .push(event);
    }));

    rt_config
}

// ── Unified fixture writing ──────────────────────────────────────────

/// Write a unified fixture to disk as gzip-compressed JSONL (`.jsonl.gz`).
///
/// Line 1: `FixtureHeader` as compact JSON.
/// Lines 2+: each `TimelineEvent` as compact JSON, one per line.
pub fn write_jsonl_fixture(
    path: &Path,
    fixture: &crate::unified_fixture::UnifiedFixture,
) -> Result<()> {
    use flate2::write::GzEncoder;
    use std::io::Write;

    let _ = std::fs::create_dir_all(path.parent().unwrap_or_else(|| std::path::Path::new(".")));
    let f = std::fs::File::create(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    let buf = std::io::BufWriter::new(f);
    let mut gz = GzEncoder::new(buf, flate2::Compression::best());

    // Line 1: header
    serde_json::to_writer(&mut gz, &fixture.header)
        .context("failed to serialize fixture header")?;
    gz.write_all(b"\n")?;

    // Lines 2+: events
    for event in &fixture.events {
        serde_json::to_writer(&mut gz, event).context("failed to serialize timeline event")?;
        gz.write_all(b"\n")?;
    }

    gz.finish()
        .with_context(|| format!("failed to finalize {}", path.display()))?;
    Ok(())
}

/// Update `config.toml` to add/update a `[fixtures].<size>` entry.
///
/// Creates the file if it doesn't exist. Preserves existing content
/// by doing a minimal TOML-aware edit: if `[fixtures]` table exists,
/// update or add the key; otherwise append the table.
pub fn update_config_toml_fixtures(
    config_path: &Path,
    size_name: &str,
    fixture_rel_path: &str,
) -> Result<()> {
    use std::io::Write;

    let content = std::fs::read_to_string(config_path).unwrap_or_default();

    // Parse as TOML value for manipulation
    let mut doc: toml::Value = if content.is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&content)
            .with_context(|| format!("failed to parse {}", config_path.display()))?
    };

    // Ensure [fixtures] table exists and set the key
    let table = doc
        .as_table_mut()
        .expect("BUG: root TOML value is not a table");
    let fixtures = table
        .entry("fixtures")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let fixtures_table = fixtures
        .as_table_mut()
        .context("[fixtures] exists but is not a table")?;
    fixtures_table.insert(
        size_name.to_owned(),
        toml::Value::String(fixture_rel_path.to_owned()),
    );

    let _ = std::fs::create_dir_all(
        config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new(".")),
    );
    let toml_str = toml::to_string_pretty(&doc).context("failed to serialize config TOML")?;
    let mut f = std::fs::File::create(config_path)
        .with_context(|| format!("failed to create {}", config_path.display()))?;
    f.write_all(toml_str.as_bytes())
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    Ok(())
}

/// Snapshot the contents of a KV directory into a `HashMap<String, String>`.
///
/// Reads each file as UTF-8 text. Skips files that can't be read as text.
#[must_use]
pub fn snapshot_kv_dir(kv_dir: &Path) -> HashMap<String, String> {
    let mut kv = HashMap::new();
    let Ok(entries) = std::fs::read_dir(kv_dir) else {
        return kv;
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Ok(value) = std::fs::read_to_string(&path) {
            kv.insert(name.to_owned(), value);
        }
    }
    kv
}

/// Convert runtime `FixtureEvent` entries to unified `TimelineEvent` entries.
#[must_use]
pub fn fixture_events_to_timeline(
    events: &[FixtureEvent],
) -> Vec<crate::unified_fixture::TimelineEvent> {
    use crate::unified_fixture::{FixtureBody, TimelineEvent, UnifiedEvent};

    events
        .iter()
        .map(|fe| {
            let event = match &fe.kind {
                FixtureEventKind::SsdpFound { search_id, data } => UnifiedEvent::SsdpFound {
                    search_id: search_id.to_wire(),
                    data: data.clone(),
                },
                FixtureEventKind::SsdpRemoved { search_id, data } => UnifiedEvent::SsdpRemoved {
                    search_id: search_id.to_wire(),
                    data: data.clone(),
                },
                FixtureEventKind::MdnsFound { browse_id, data } => UnifiedEvent::MdnsFound {
                    browse_id: browse_id.to_wire(),
                    data: data.clone(),
                },
                FixtureEventKind::MdnsRemoved { browse_id, data } => UnifiedEvent::MdnsRemoved {
                    browse_id: browse_id.to_wire(),
                    data: data.clone(),
                },
                FixtureEventKind::WsOpen { ws_id } => UnifiedEvent::WsOpen {
                    ws_id: ws_id.to_wire(),
                },
                FixtureEventKind::WsMessage { ws_id, data } => UnifiedEvent::WsMessage {
                    ws_id: ws_id.to_wire(),
                    data: FixtureBody::from_bytes(data),
                },
                FixtureEventKind::WsClose { ws_id, code } => UnifiedEvent::WsClose {
                    ws_id: ws_id.to_wire(),
                    code: *code,
                },
                FixtureEventKind::SocketConnected { socket_id } => UnifiedEvent::SocketConnected {
                    socket_id: socket_id.to_wire(),
                },
                FixtureEventKind::SocketData { socket_id, data } => UnifiedEvent::SocketData {
                    socket_id: socket_id.to_wire(),
                    data: FixtureBody::from_bytes(data),
                },
                FixtureEventKind::SocketClosed { socket_id, code } => UnifiedEvent::SocketClosed {
                    socket_id: socket_id.to_wire(),
                    code: *code,
                },
                FixtureEventKind::UdpResponse {
                    broadcast_id,
                    data,
                    source,
                } => UnifiedEvent::UdpResponse {
                    broadcast_id: broadcast_id.to_wire(),
                    data: data.clone(),
                    source: source.clone(),
                },
                FixtureEventKind::AudioPlay {
                    sound_id,
                    volume,
                    name,
                    duration_ms,
                } => UnifiedEvent::AudioPlay {
                    sound_id: u32::from(sound_id.to_wire()),
                    volume: *volume,
                    name: name.clone(),
                    duration_ms: *duration_ms,
                },
                FixtureEventKind::LedSetEffect {
                    effect,
                    r,
                    g,
                    b,
                    period_ms,
                    duration_ms,
                } => UnifiedEvent::LedSetEffect {
                    effect: *effect,
                    r: *r,
                    g: *g,
                    b: *b,
                    period_ms: *period_ms,
                    duration_ms: *duration_ms,
                },
                FixtureEventKind::LedEnable => UnifiedEvent::LedEnable,
                FixtureEventKind::LedDisable => UnifiedEvent::LedDisable,
            };
            TimelineEvent {
                at_ms: fe.at_ms,
                event,
            }
        })
        .collect()
}

// ── Shared helpers (used by both capture and testbed) ────────────────

/// Walk up from WASM file to find the widget crate root (has `Cargo.toml`).
///
/// Finds the widget source directory from its WASM binary path.
///
/// The WASM binary lives in a shared workspace target:
///   `examples/target/wasm32-unknown-unknown/release/hello_widget.wasm`
///
/// Strategy: derive the crate name from the filename (underscores → hyphens),
/// walk up to find the workspace root (directory containing `Cargo.toml`),
/// then resolve `{workspace_root}/{crate_name}/`.
#[must_use]
pub fn find_widget_root(wasm_path: &Path) -> Option<PathBuf> {
    let mut dir = wasm_path.parent();
    for _ in 0..6 {
        let Some(d) = dir else { break };
        if d.join("Cargo.toml").exists() {
            return Some(d.to_owned());
        }
        dir = d.parent();
    }
    None
}

/// Load `secrets.ini` from the widget directory and seed KV store files.
///
/// Walks up from the WASM binary looking for `secrets.ini` (max 6 levels).
/// Parses `KEY=VALUE` lines and writes each as a file in `kv_dir`.
/// Does not overwrite existing KV files (config/variant overrides take precedence).
pub fn seed_kv_from_secrets(wasm_path: &Path, kv_dir: &Path) {
    // Resolve the widget crate root, then look for secrets.ini there.
    let Some(widget_root) = find_widget_root(wasm_path) else {
        return;
    };
    let path = widget_root.join("secrets.ini");
    if !path.exists() {
        return;
    }

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("failed to read {}: {e}", path.display());
            return;
        }
    };

    let _ = std::fs::create_dir_all(kv_dir);
    let mut count = 0_u32;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() {
            continue;
        }
        let kv_file = kv_dir.join(key);
        // Only seed if the key doesn't already exist (don't overwrite runtime changes)
        if !kv_file.exists() {
            if let Err(e) = std::fs::write(&kv_file, value.as_bytes()) {
                tracing::warn!("failed to write KV {key}: {e}");
            } else {
                count += 1;
            }
        }
    }
    if count > 0 {
        tracing::info!("seeded {count} KV key(s) from {}", path.display());
    }
}
