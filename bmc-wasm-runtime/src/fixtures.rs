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

//! Shared fixture utilities for capture and testbed binaries.
//!
//! Provides unified fixture recording, writing, and KV seeding for
//! headless capture and interactive testbed sessions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};

use crate::{FixtureEvent, FixtureEventKind, RuntimeConfig};

#[derive(Debug)]
pub struct PreparedWidget {
    wasm_path: PathBuf,
    asset_root: PathBuf,
    _temporary_directory: Option<tempfile::TempDir>,
}

impl PreparedWidget {
    pub fn new(source_wasm: &Path, asset_root: Option<&Path>) -> Result<Self> {
        if let Some(asset_root) = asset_root {
            ensure!(
                asset_root.is_dir(),
                "package asset root is not a directory: {}",
                asset_root.display()
            );
            return Ok(Self {
                wasm_path: source_wasm.to_owned(),
                asset_root: asset_root.to_owned(),
                _temporary_directory: None,
            });
        }

        let temporary_directory = tempfile::Builder::new()
            .prefix("bmc-widget-")
            .tempdir()
            .context("create temporary prepared widget directory")?;
        let wasm_path = temporary_directory.path().join("widget.wasm");
        let asset_root = temporary_directory.path().join("assets");
        let artifact_root = source_wasm
            .parent()
            .map(|directory| {
                if directory.file_name().is_some_and(|name| name == "deps") {
                    directory.to_owned()
                } else {
                    directory.join("deps")
                }
            })
            .filter(|directory| directory.is_dir());
        bmc_wasm_assets::extract_package_assets(
            source_wasm,
            artifact_root.as_deref(),
            &wasm_path,
            &asset_root,
        )
        .with_context(|| format!("prepare package assets from {}", source_wasm.display()))?;

        Ok(Self {
            wasm_path,
            asset_root,
            _temporary_directory: Some(temporary_directory),
        })
    }

    #[must_use]
    pub fn wasm_path(&self) -> &Path {
        &self.wasm_path
    }

    #[must_use]
    pub fn asset_root(&self) -> &Path {
        &self.asset_root
    }
}

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
/// Creates the file if it doesn't exist.
/// Edits in place with `toml_edit` so the file's comments
/// and key ordering survive — a plain `toml::Value`
/// round-trip drops both.
pub fn update_config_toml_fixtures(
    config_path: &Path,
    size_name: &str,
    fixture_rel_path: &str,
) -> Result<()> {
    let content = std::fs::read_to_string(config_path).unwrap_or_default();
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("failed to parse {}", config_path.display()))?;
    doc["fixtures"][size_name] = toml_edit::value(fixture_rel_path);

    if let Some(parent) = config_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(config_path, doc.to_string())
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
    use crate::unified_fixture::TimelineEvent;

    events
        .iter()
        .map(|fe| TimelineEvent {
            at_ms: fe.at_ms,
            event: fixture_event_kind_to_unified(&fe.kind),
        })
        .collect()
}

fn fixture_event_kind_to_unified(kind: &FixtureEventKind) -> crate::unified_fixture::UnifiedEvent {
    use crate::unified_fixture::{FixtureBody, UnifiedEvent};

    match kind {
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
        FixtureEventKind::LedSetEndless {
            effect,
            r,
            g,
            b,
            period_ms,
            scope,
        } => UnifiedEvent::LedSetEndless {
            effect: *effect,
            r: *r,
            g: *g,
            b: *b,
            period_ms: *period_ms,
            scope: *scope,
        },
        FixtureEventKind::LedSetTemporary {
            effect,
            r,
            g,
            b,
            period_ms,
            duration_ms,
            scope,
        } => UnifiedEvent::LedSetTemporary {
            effect: *effect,
            r: *r,
            g: *g,
            b: *b,
            period_ms: *period_ms,
            duration_ms: *duration_ms,
            scope: *scope,
        },
        FixtureEventKind::LedStop => UnifiedEvent::LedStop,
    }
}

// ── Shared helpers (used by both capture and testbed) ────────────────

/// Walk up from WASM file to find the widget crate root.
///
/// The WASM binary lives in a shared workspace target:
///   `examples/target/wasm32-unknown-unknown/release/hello_widget.wasm`
///
/// Strategy: derive the crate name from the filename (underscores → hyphens),
/// walk up looking for `{ancestor}/{crate_name}/Cargo.toml`. Returning the
/// *workspace* root (the first `Cargo.toml` encountered) is wrong — fixtures
/// would land at `examples/capture/` instead of `examples/<widget>/capture/`.
#[must_use]
pub fn find_widget_root(wasm_path: &Path) -> Option<PathBuf> {
    let stem = wasm_path.file_stem()?.to_str()?;
    let crate_name = stem.replace('_', "-");
    let mut dir = wasm_path.parent();
    for _ in 0..6 {
        let Some(d) = dir else { break };
        let candidate = d.join(&crate_name);
        if candidate.join("Cargo.toml").exists() {
            return Some(candidate);
        }
        dir = d.parent();
    }
    None
}

pub fn seed_kv_from_widget_root(widget_root: &Path, kv_dir: &Path) {
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

#[cfg(test)]
mod tests {
    use std::fs;

    use bmc_wasm_assets::{Records, contains_package_asset_section, encode_record};
    use bmc_wasm_protocol::{PACKAGE_ASSET_SECTION_NAME, PackageAssetKind, PackageAssetRef};

    use super::PreparedWidget;

    #[test]
    fn prepared_widget_extracts_assets_before_runtime_load() {
        let record = encode_record(PackageAssetKind::Bitmap, "cover", b"encoded-image")
            .expect("encode package asset fixture");
        let id = Records::new(&record)
            .next()
            .expect("one package asset record")
            .expect("parse package asset fixture")
            .id;
        let mut compiler_wasm = module_with_custom_section(PACKAGE_ASSET_SECTION_NAME, &record);
        append_passive_data_segment(
            &mut compiler_wasm,
            PackageAssetRef::new(PackageAssetKind::Bitmap, id).as_bytes(),
        );
        let directory = tempfile::tempdir().expect("create compiler output directory");
        let input = directory.path().join("widget.wasm");
        fs::write(&input, compiler_wasm).expect("write compiler output");

        let prepared = PreparedWidget::new(&input, None).expect("prepare widget package");

        let runtime_wasm = fs::read(prepared.wasm_path()).expect("read runtime module");
        assert!(
            !contains_package_asset_section(&runtime_wasm).expect("inspect runtime module"),
            "runtime module must not retain package asset records"
        );
        assert_eq!(
            fs::read(
                prepared
                    .asset_root()
                    .join("v1/bitmap")
                    .join(format!("{id}.asset"))
            )
            .expect("read extracted package asset"),
            b"encoded-image"
        );
    }

    fn module_with_custom_section(name: &str, data: &[u8]) -> Vec<u8> {
        let mut module = b"\0asm\x01\0\0\0".to_vec();
        let mut payload = Vec::new();
        encode_u32_leb(&mut payload, name.len());
        payload.extend_from_slice(name.as_bytes());
        payload.extend_from_slice(data);
        module.push(0);
        encode_u32_leb(&mut module, payload.len());
        module.extend_from_slice(&payload);
        module
    }

    fn append_passive_data_segment(module: &mut Vec<u8>, data: &[u8]) {
        let mut payload = Vec::new();
        encode_u32_leb(&mut payload, 1);
        encode_u32_leb(&mut payload, 1);
        encode_u32_leb(&mut payload, data.len());
        payload.extend_from_slice(data);
        module.push(11);
        encode_u32_leb(module, payload.len());
        module.extend_from_slice(&payload);
    }

    fn encode_u32_leb(output: &mut Vec<u8>, value: usize) {
        let mut remaining = u32::try_from(value).expect("fixture length fits in u32");
        loop {
            let mut byte =
                u8::try_from(remaining & 0x7f).expect("BUG: seven bits always fit in a byte");
            remaining >>= 7;
            if remaining != 0 {
                byte |= 0x80;
            }
            output.push(byte);
            if remaining == 0 {
                break;
            }
        }
    }
}
