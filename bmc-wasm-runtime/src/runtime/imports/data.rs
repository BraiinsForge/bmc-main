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

//! Data-, persistence-, and formatting-focused guest imports.

use anyhow::{Result, bail};
use bmc_wasm_protocol::system::{NumberFormat, TemperatureUnit, UnitSystem};
use bmc_wasm_protocol::{JsonId, XmlId};
use chrono::{DateTime, Utc};
use wasmi::{Caller, Extern, Linker};

use crate::host_api::HostState;
use crate::xml::XmlDocumentIndex;

use super::super::memory::{read_bytes, read_string, write_to_wasm};
use super::super::time::{expand_rrule_impl, format_number_with_prefs, tz_convert_impl};

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    register_kv_write_imports(linker)?;
    register_kv_get_import(linker)?;
    register_cache_imports(linker)?;
    register_log_import(linker)?;
    register_json_parse_import(linker)?;
    register_json_string_import(linker)?;
    register_json_numeric_imports(linker)?;
    register_json_bool_import(linker)?;
    register_date_imports(linker)?;
    register_xml_imports(linker)?;
    register_number_format_import(linker)?;
    register_speed_format_import(linker)?;
    register_distance_format_import(linker)?;
    register_temperature_format_import(linker)?;
    register_rrule_import(linker)?;
    register_timezone_import(linker)?;
    register_fuel_import(linker)?;
    register_profile_section_import(linker)?;
    Ok(())
}

fn register_kv_write_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_kv_set",
        |mut caller: Caller<'_, HostState>,
         key_ptr: u32,
         key_len: u32,
         val_ptr: u32,
         val_len: u32| {
            let key = read_string(&caller, key_ptr, key_len);
            let val = read_bytes(&caller, val_ptr, val_len);
            let (Some(key), Some(val)) = (key, val) else {
                return;
            };
            if let Err(e) = validate_kv_key(&key) {
                tracing::warn!("kv_set rejected key: {e}");
                return;
            }

            let state = caller.data_mut();
            state.kv_cache.insert(key.clone(), val.clone());

            if let Some(ref base) = state.kv_store_path {
                let dir = base.clone();
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    tracing::warn!("kv_set: failed to create dir: {e}");
                    return;
                }
                let path = match kv_disk_path(&dir, &key) {
                    Ok(path) => path,
                    Err(e) => {
                        tracing::warn!("kv_set rejected key: {e}");
                        return;
                    }
                };
                if let Err(e) = std::fs::write(&path, &val) {
                    tracing::warn!("kv_set: failed to write {}: {e}", path.display());
                }
            }
        },
    )?;

    linker.func_wrap(
        "env",
        "host_kv_delete",
        |mut caller: Caller<'_, HostState>, key_ptr: u32, key_len: u32| {
            let Some(key) = read_string(&caller, key_ptr, key_len) else {
                return;
            };
            if let Err(e) = validate_kv_key(&key) {
                tracing::warn!("kv_delete rejected key: {e}");
                return;
            }

            let state = caller.data_mut();
            state.kv_cache.remove(&key);

            if let Some(ref base) = state.kv_store_path {
                let path = match kv_disk_path(base, &key) {
                    Ok(path) => path,
                    Err(e) => {
                        tracing::warn!("kv_delete rejected key: {e}");
                        return;
                    }
                };
                let _ = std::fs::remove_file(&path);
            }
        },
    )?;

    Ok(())
}

fn register_kv_get_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_kv_get",
        |mut caller: Caller<'_, HostState>,
         key_ptr: u32,
         key_len: u32,
         out_ptr: u32,
         out_cap: u32|
         -> i32 {
            let Some(key) = read_string(&caller, key_ptr, key_len) else {
                return -1;
            };
            if let Err(e) = validate_kv_key(&key) {
                tracing::warn!("kv_get rejected key: {e}");
                return -1;
            }

            let state = caller.data_mut();

            if let Some(val) = state.kv_cache.get(&key) {
                let val_len = i32::try_from(val.len()).unwrap_or(i32::MAX);
                if out_cap > 0 && out_cap as usize >= val.len() {
                    let val = val.clone();
                    let memory = caller.get_export("memory").and_then(Extern::into_memory);
                    if let Some(memory) = memory {
                        let mem = memory.data_mut(&mut caller);
                        let start = out_ptr as usize;
                        let end = start + val.len();
                        if end <= mem.len() {
                            mem[start..end].copy_from_slice(&val);
                        }
                    }
                }
                return val_len;
            }

            if let Some(ref base) = state.kv_store_path.clone() {
                let path = match kv_disk_path(base, &key) {
                    Ok(path) => path,
                    Err(e) => {
                        tracing::warn!("kv_get rejected key: {e}");
                        return -1;
                    }
                };
                if let Ok(val) = std::fs::read(&path) {
                    let val_len = i32::try_from(val.len()).unwrap_or(i32::MAX);
                    state.kv_cache.insert(key, val.clone());
                    if out_cap > 0 && out_cap as usize >= val.len() {
                        let memory = caller.get_export("memory").and_then(Extern::into_memory);
                        if let Some(memory) = memory {
                            let mem = memory.data_mut(&mut caller);
                            let start = out_ptr as usize;
                            let end = start + val.len();
                            if end <= mem.len() {
                                mem[start..end].copy_from_slice(&val);
                            }
                        }
                    }
                    return val_len;
                }
            }

            -1
        },
    )?;

    Ok(())
}

// Flash blob cache, curried per widget instance; `get` returns the `DiskCache` record verbatim.
#[expect(
    clippy::too_many_lines,
    reason = "the paired cache put/get imports register most clearly together"
)]
fn register_cache_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_cache_put",
        |caller: Caller<'_, HostState>,
         tag_ptr: u32,
         tag_len: u32,
         meta_ptr: u32,
         meta_len: u32,
         bytes_ptr: u32,
         bytes_len: u32| {
            let Some(tag) = read_string(&caller, tag_ptr, tag_len) else {
                return;
            };
            if let Err(e) = validate_kv_key(&tag) {
                tracing::warn!("cache_put rejected tag: {e}");
                return;
            }
            let state = caller.data();
            let Some(cache) = state.asset_cache.as_ref() else {
                return;
            };
            // Reject an over-cap entry by its declared sizes before copying the
            // payload out of guest memory.
            if !cache.accepts_entry(meta_len as usize, bytes_len as usize) {
                tracing::warn!(
                    meta_len,
                    bytes_len,
                    "cache_put rejected: entry over bucket cap"
                );
                return;
            }
            let (Some(meta), Some(bytes)) = (
                read_bytes(&caller, meta_ptr, meta_len),
                read_bytes(&caller, bytes_ptr, bytes_len),
            ) else {
                return;
            };
            // `saved_at` rides the injected clock so hermetic replay is deterministic.
            let saved_at = u64::try_from(state.system_time.timestamp_millis()).unwrap_or(0);
            if let Err(e) = cache.put(&tag, saved_at, &meta, &bytes) {
                tracing::warn!("cache_put failed for {tag}: {e}");
            }
        },
    )?;

    linker.func_wrap(
        "env",
        "host_cache_get",
        |mut caller: Caller<'_, HostState>,
         tag_ptr: u32,
         tag_len: u32,
         out_ptr: u32,
         out_cap: u32|
         -> i32 {
            let Some(tag) = read_string(&caller, tag_ptr, tag_len) else {
                return -1;
            };
            if let Err(e) = validate_kv_key(&tag) {
                tracing::warn!("cache_get rejected tag: {e}");
                return -1;
            }
            let record = {
                let state = caller.data();
                let Some(cache) = state.asset_cache.as_ref() else {
                    return -1;
                };
                let Some(blob) = cache.get(&tag) else {
                    return -1;
                };
                let meta = blob.metadata();
                let bytes = blob.bytes();
                let mut record = Vec::with_capacity(12 + meta.len() + bytes.len());
                record.extend_from_slice(&blob.saved_at.to_le_bytes());
                let meta_len = u32::try_from(meta.len()).unwrap_or(u32::MAX);
                record.extend_from_slice(&meta_len.to_le_bytes());
                record.extend_from_slice(meta);
                record.extend_from_slice(bytes);
                record
            };

            let needed = i32::try_from(record.len()).unwrap_or(i32::MAX);
            if out_cap == 0 || (out_cap as usize) < record.len() {
                return needed;
            }
            let memory = caller.get_export("memory").and_then(Extern::into_memory);
            if let Some(memory) = memory {
                let mem = memory.data_mut(&mut caller);
                let start = out_ptr as usize;
                if let Some(end) = start.checked_add(record.len())
                    && end <= mem.len()
                {
                    mem[start..end].copy_from_slice(&record);
                }
            }
            needed
        },
    )?;

    linker.func_wrap(
        "env",
        "host_cache_evict",
        |caller: Caller<'_, HostState>, tag_ptr: u32, tag_len: u32| {
            let Some(tag) = read_string(&caller, tag_ptr, tag_len) else {
                return;
            };
            if validate_kv_key(&tag).is_err() {
                return;
            }
            let state = caller.data();
            if let Some(cache) = state.asset_cache.as_ref() {
                cache.evict(&tag);
            }
        },
    )?;

    register_cache_stat_import(linker)?;

    Ok(())
}

// Freshness peek: returns `[saved_at u64 | metadata]` with no payload bytes, so a
// widget can check identity + TTL without pulling the blob into wasm.
fn register_cache_stat_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_cache_stat",
        |mut caller: Caller<'_, HostState>,
         tag_ptr: u32,
         tag_len: u32,
         out_ptr: u32,
         out_cap: u32|
         -> i32 {
            let Some(tag) = read_string(&caller, tag_ptr, tag_len) else {
                return -1;
            };
            if let Err(e) = validate_kv_key(&tag) {
                tracing::warn!("cache_stat rejected tag: {e}");
                return -1;
            }
            let record = {
                let state = caller.data();
                let Some(cache) = state.asset_cache.as_ref() else {
                    return -1;
                };
                let Some(blob) = cache.get(&tag) else {
                    return -1;
                };
                let meta = blob.metadata();
                let mut record = Vec::with_capacity(8 + meta.len());
                record.extend_from_slice(&blob.saved_at.to_le_bytes());
                record.extend_from_slice(meta);
                record
            };
            let needed = i32::try_from(record.len()).unwrap_or(i32::MAX);
            if out_cap == 0 || (out_cap as usize) < record.len() {
                return needed;
            }
            let memory = caller.get_export("memory").and_then(Extern::into_memory);
            if let Some(memory) = memory {
                let mem = memory.data_mut(&mut caller);
                let start = out_ptr as usize;
                if let Some(end) = start.checked_add(record.len())
                    && end <= mem.len()
                {
                    mem[start..end].copy_from_slice(&record);
                }
            }
            needed
        },
    )?;
    Ok(())
}

fn register_log_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_log",
        |caller: Caller<'_, HostState>, ptr: u32, len: u32, level: u32| {
            let Some(msg) = read_string(&caller, ptr, len) else {
                return;
            };
            match level {
                0 => tracing::debug!("{msg}"),
                1 => tracing::info!("{msg}"),
                2 => tracing::warn!("{msg}"),
                _ => tracing::error!("{msg}"),
            }
        },
    )?;

    Ok(())
}

fn register_json_parse_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_json_parse",
        |mut caller: Caller<'_, HostState>, body_ptr: u32, body_len: u32| -> u32 {
            let Some(bytes) = read_bytes(&caller, body_ptr, body_len) else {
                return 0;
            };

            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                return 0;
            };

            let state = caller.data_mut();
            let doc_id = JsonId::alloc(&mut state.next_json_id);
            state.json_docs.insert(doc_id, value);
            doc_id.to_wire()
        },
    )?;

    linker.func_wrap(
        "env",
        "host_json_free",
        |mut caller: Caller<'_, HostState>, doc_id: u32| {
            let Some(doc_id) = JsonId::from_wire(doc_id) else {
                return;
            };
            caller.data_mut().json_docs.remove(&doc_id);
        },
    )?;

    Ok(())
}

fn register_json_string_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_json_get_str",
        |mut caller: Caller<'_, HostState>,
         doc_id: u32,
         path_ptr: u32,
         path_len: u32,
         out_ptr: u32,
         out_len: u32|
         -> i32 {
            let Some(path) = read_string(&caller, path_ptr, path_len) else {
                return -1;
            };
            let Some(doc_id) = JsonId::from_wire(doc_id) else {
                return -1;
            };

            let result = {
                let state = caller.data();
                let Some(doc) = state.json_docs.get(&doc_id) else {
                    return -1;
                };
                let Some(val) = doc.pointer(&path) else {
                    return -1;
                };
                let Some(s) = val.as_str() else {
                    return -2;
                };
                s.to_owned()
            };

            write_to_wasm(&mut caller, &result, out_ptr, out_len)
        },
    )?;

    Ok(())
}

fn register_json_numeric_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_json_get_i64",
        |caller: Caller<'_, HostState>, doc_id: u32, path_ptr: u32, path_len: u32| -> i64 {
            let Some(path) = read_string(&caller, path_ptr, path_len) else {
                return i64::MIN;
            };
            let Some(doc_id) = JsonId::from_wire(doc_id) else {
                return i64::MIN;
            };
            let state = caller.data();
            let Some(doc) = state.json_docs.get(&doc_id) else {
                return i64::MIN;
            };
            let Some(val) = doc.pointer(&path) else {
                return i64::MIN;
            };
            val.as_i64().unwrap_or(i64::MIN)
        },
    )?;

    linker.func_wrap(
        "env",
        "host_json_get_f64",
        |caller: Caller<'_, HostState>, doc_id: u32, path_ptr: u32, path_len: u32| -> f64 {
            let Some(path) = read_string(&caller, path_ptr, path_len) else {
                return f64::NAN;
            };
            let Some(doc_id) = JsonId::from_wire(doc_id) else {
                return f64::NAN;
            };
            let state = caller.data();
            let Some(doc) = state.json_docs.get(&doc_id) else {
                return f64::NAN;
            };
            let Some(val) = doc.pointer(&path) else {
                return f64::NAN;
            };
            val.as_f64().unwrap_or(f64::NAN)
        },
    )?;

    Ok(())
}

fn register_json_bool_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_json_get_bool",
        |caller: Caller<'_, HostState>, doc_id: u32, path_ptr: u32, path_len: u32| -> i32 {
            let Some(path) = read_string(&caller, path_ptr, path_len) else {
                return -1;
            };
            let Some(doc_id) = JsonId::from_wire(doc_id) else {
                return -1;
            };
            let state = caller.data();
            let Some(doc) = state.json_docs.get(&doc_id) else {
                return -1;
            };
            let Some(val) = doc.pointer(&path) else {
                return -1;
            };
            match val.as_bool() {
                Some(true) => 1,
                Some(false) => 0,
                None => -1,
            }
        },
    )?;

    Ok(())
}

fn register_date_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_parse_date",
        |caller: Caller<'_, HostState>, str_ptr: u32, str_len: u32| -> i64 {
            let Some(s) = read_string(&caller, str_ptr, str_len) else {
                return i64::MIN;
            };
            s.parse::<DateTime<Utc>>()
                .map_or(i64::MIN, |dt| dt.timestamp())
        },
    )?;

    linker.func_wrap(
        "env",
        "host_format_date",
        |mut caller: Caller<'_, HostState>,
         timestamp: i64,
         fmt_ptr: u32,
         fmt_len: u32,
         out_ptr: u32,
         out_len: u32|
         -> i32 {
            let Some(fmt) = read_string(&caller, fmt_ptr, fmt_len) else {
                return -1;
            };
            let Some(dt) = DateTime::<Utc>::from_timestamp(timestamp, 0) else {
                return -1;
            };
            let formatted = dt.format(&fmt).to_string();
            write_to_wasm(&mut caller, &formatted, out_ptr, out_len)
        },
    )?;

    Ok(())
}

fn register_xml_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_xml_parse",
        |mut caller: Caller<'_, HostState>, body_ptr: u32, body_len: u32| -> u32 {
            let Some(bytes) = read_bytes(&caller, body_ptr, body_len) else {
                return 0;
            };

            let Ok(xml_str) = String::from_utf8(bytes) else {
                return 0;
            };

            let Ok(xml_index) = XmlDocumentIndex::from_xml(&xml_str) else {
                return 0;
            };

            let state = caller.data_mut();
            let doc_id = XmlId::alloc(&mut state.next_xml_id);
            state.xml_indices.insert(doc_id, xml_index);
            doc_id.to_wire()
        },
    )?;

    linker.func_wrap(
        "env",
        "host_xml_get_str",
        |mut caller: Caller<'_, HostState>,
         doc_id: u32,
         path_ptr: u32,
         path_len: u32,
         out_ptr: u32,
         out_len: u32|
         -> i32 {
            let Some(path) = read_string(&caller, path_ptr, path_len) else {
                return -1;
            };
            let Some(doc_id) = XmlId::from_wire(doc_id) else {
                return -1;
            };

            let result = {
                let state = caller.data();
                xml_lookup_text(&state.xml_indices, doc_id, &path)
            };

            let Some(text) = result else {
                return -1;
            };
            write_to_wasm(&mut caller, &text, out_ptr, out_len)
        },
    )?;

    linker.func_wrap(
        "env",
        "host_xml_get_f64",
        |caller: Caller<'_, HostState>, doc_id: u32, path_ptr: u32, path_len: u32| -> f64 {
            let Some(path) = read_string(&caller, path_ptr, path_len) else {
                return f64::NAN;
            };
            let Some(doc_id) = XmlId::from_wire(doc_id) else {
                return f64::NAN;
            };

            let state = caller.data();
            xml_lookup_text(&state.xml_indices, doc_id, &path)
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(f64::NAN)
        },
    )?;

    linker.func_wrap(
        "env",
        "host_xml_free",
        |mut caller: Caller<'_, HostState>, doc_id: u32| {
            let Some(doc_id) = XmlId::from_wire(doc_id) else {
                return;
            };
            caller.data_mut().xml_indices.remove(&doc_id);
        },
    )?;

    Ok(())
}

fn register_number_format_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_format_number",
        |mut caller: Caller<'_, HostState>,
         value: f64,
         decimals: u32,
         out_ptr: u32,
         out_len: u32|
         -> i32 {
            let number_format = caller.data().system.snapshot().settings.number_format;
            let formatted = format_number_with_prefs(number_format, value, decimals);
            write_to_wasm(&mut caller, &formatted, out_ptr, out_len)
        },
    )?;

    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MetricSpeedUnit {
    KmH,
    Ms,
}

impl From<u32> for MetricSpeedUnit {
    fn from(tag: u32) -> Self {
        match tag {
            1 => Self::Ms,
            _ => Self::KmH,
        }
    }
}

fn format_speed_with_prefs(
    number_format: NumberFormat,
    unit_system: UnitSystem,
    value_kmh: f64,
    decimals: u32,
    metric_unit: MetricSpeedUnit,
) -> String {
    let metric_unit = match metric_unit {
        MetricSpeedUnit::KmH => bmc_shared_utils::unit_system::MetricSpeedUnit::KmH,
        MetricSpeedUnit::Ms => bmc_shared_utils::unit_system::MetricSpeedUnit::Ms,
    };
    bmc_shared_utils::unit_system::UnitSystem::from(unit_system).format_speed(
        number_format.into(),
        value_kmh,
        decimals as usize,
        metric_unit,
    )
}

fn register_speed_format_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_format_speed",
        |mut caller: Caller<'_, HostState>,
         value: f64,
         decimals: u32,
         metric_unit: u32,
         out_ptr: u32,
         out_len: u32|
         -> i32 {
            // Pull the two `Copy` enums out into locals so the snapshot borrow
            // drops before `format_number_with_prefs` allocates — avoids cloning
            // the timezone String on every speed-format call.
            let (number_format, unit_system) = {
                let s = caller.data().system.snapshot();
                (s.settings.number_format, s.settings.unit_system)
            };
            let formatted = format_speed_with_prefs(
                number_format,
                unit_system,
                value,
                decimals,
                MetricSpeedUnit::from(metric_unit),
            );
            write_to_wasm(&mut caller, &formatted, out_ptr, out_len)
        },
    )?;

    Ok(())
}

#[cfg(test)]
mod speed_format_tests {
    use super::*;

    #[test]
    fn metric_kmh_keeps_value_and_suffix() {
        let s = format_speed_with_prefs(
            NumberFormat::SpaceGroupCommaDecimal,
            UnitSystem::Metric,
            12.6,
            1,
            MetricSpeedUnit::KmH,
        );
        assert_eq!(s, "12,6 km/h");
    }

    #[test]
    fn metric_ms_divides_by_3_6_and_labels_ms() {
        // 12.6 km/h -> 3.5 m/s
        let s = format_speed_with_prefs(
            NumberFormat::SpaceGroupCommaDecimal,
            UnitSystem::Metric,
            12.6,
            1,
            MetricSpeedUnit::Ms,
        );
        assert_eq!(s, "3,5 m/s");
    }

    #[test]
    fn imperial_is_mph_regardless_of_metric_unit() {
        let kmh = format_speed_with_prefs(
            NumberFormat::SpaceGroupCommaDecimal,
            UnitSystem::Imperial,
            100.0,
            0,
            MetricSpeedUnit::KmH,
        );
        let ms = format_speed_with_prefs(
            NumberFormat::SpaceGroupCommaDecimal,
            UnitSystem::Imperial,
            100.0,
            0,
            MetricSpeedUnit::Ms,
        );
        assert_eq!(kmh, "62 mph");
        assert_eq!(ms, "62 mph");
    }
}

fn format_distance_with_prefs(
    number_format: NumberFormat,
    unit_system: UnitSystem,
    value_km: f64,
    decimals: u32,
) -> String {
    bmc_shared_utils::unit_system::UnitSystem::from(unit_system).format_distance(
        number_format.into(),
        value_km,
        decimals as usize,
    )
}

fn register_distance_format_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_format_distance",
        |mut caller: Caller<'_, HostState>,
         value: f64,
         decimals: u32,
         out_ptr: u32,
         out_len: u32|
         -> i32 {
            let (number_format, unit_system) = {
                let s = caller.data().system.snapshot();
                (s.settings.number_format, s.settings.unit_system)
            };
            let formatted = format_distance_with_prefs(number_format, unit_system, value, decimals);
            write_to_wasm(&mut caller, &formatted, out_ptr, out_len)
        },
    )?;

    Ok(())
}

#[cfg(test)]
mod distance_format_tests {
    use super::*;

    #[test]
    fn metric_keeps_kilometers() {
        let s = format_distance_with_prefs(
            NumberFormat::SpaceGroupCommaDecimal,
            UnitSystem::Metric,
            420.0,
            0,
        );
        assert_eq!(s, "420 km");
    }

    #[test]
    fn imperial_converts_to_miles() {
        // 420 km -> ~261 mi.
        let s = format_distance_with_prefs(
            NumberFormat::SpaceGroupCommaDecimal,
            UnitSystem::Imperial,
            420.0,
            0,
        );
        assert_eq!(s, "261 mi");
    }
}

fn format_temperature_with_prefs(
    number_format: NumberFormat,
    temperature_unit: TemperatureUnit,
    value_c: f64,
    decimals: u32,
    show_unit: bool,
) -> String {
    bmc_shared_utils::temperature::TemperatureUnit::from(temperature_unit).format(
        number_format.into(),
        value_c,
        decimals as usize,
        show_unit,
    )
}

fn register_temperature_format_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_format_temperature",
        |mut caller: Caller<'_, HostState>,
         value: f64,
         decimals: u32,
         show_unit: u32,
         out_ptr: u32,
         out_len: u32|
         -> i32 {
            let (number_format, temperature_unit) = {
                let s = caller.data().system.snapshot();
                (s.settings.number_format, s.settings.temperature_unit)
            };
            let formatted = format_temperature_with_prefs(
                number_format,
                temperature_unit,
                value,
                decimals,
                show_unit != 0,
            );
            write_to_wasm(&mut caller, &formatted, out_ptr, out_len)
        },
    )?;

    Ok(())
}

#[cfg(test)]
mod temperature_format_tests {
    use super::*;

    #[test]
    fn celsius_with_unit_keeps_scale_letter_and_space() {
        let s = format_temperature_with_prefs(
            NumberFormat::SpaceGroupCommaDecimal,
            TemperatureUnit::Celsius,
            20.5,
            1,
            true,
        );
        assert_eq!(s, "20,5 \u{00b0}C");
    }

    #[test]
    fn fahrenheit_converts_and_labels() {
        let s = format_temperature_with_prefs(
            NumberFormat::SpaceGroupCommaDecimal,
            TemperatureUnit::Fahrenheit,
            20.0,
            0,
            true,
        );
        assert_eq!(s, "68 \u{00b0}F");
    }

    #[test]
    fn degree_only_drops_scale_letter_and_space() {
        let celsius = format_temperature_with_prefs(
            NumberFormat::SpaceGroupCommaDecimal,
            TemperatureUnit::Celsius,
            20.0,
            0,
            false,
        );
        let fahrenheit = format_temperature_with_prefs(
            NumberFormat::SpaceGroupCommaDecimal,
            TemperatureUnit::Fahrenheit,
            20.0,
            0,
            false,
        );
        assert_eq!(celsius, "20\u{00b0}");
        assert_eq!(fahrenheit, "68\u{00b0}");
    }
}

fn register_rrule_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_expand_rrule",
        |mut caller: Caller<'_, HostState>,
         input_ptr: u32,
         input_len: u32,
         out_ptr: u32,
         out_cap: u32|
         -> i32 {
            let Some(input_bytes) = read_bytes(&caller, input_ptr, input_len) else {
                return -1;
            };

            let timestamps = expand_rrule_impl(&input_bytes);
            let needed = timestamps.len() * 8;
            let needed_i32 = i32::try_from(needed).unwrap_or(i32::MAX);

            if out_cap == 0 {
                return needed_i32;
            }

            if (out_cap as usize) < needed {
                return needed_i32;
            }

            let memory = caller.get_export("memory").and_then(Extern::into_memory);
            if let Some(memory) = memory {
                let data = memory.data_mut(&mut caller);
                let start = out_ptr as usize;
                for (i, &ts) in timestamps.iter().enumerate() {
                    let offset = start + i * 8;
                    if offset + 8 <= data.len() {
                        data[offset..offset + 8].copy_from_slice(&ts.to_le_bytes());
                    }
                }
            }
            needed_i32
        },
    )?;

    Ok(())
}

// Remaining guest fuel (instruction budget) for the current frame. The SDK's
// `profile` facility reads this around a span to attribute instruction count —
// a hardware-independent cost — to a section. Always registered; widgets only
// import it when built with the SDK `profiling` feature.
fn register_fuel_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_fuel_remaining",
        |caller: Caller<'_, HostState>| -> u64 { caller.get_fuel().unwrap_or(0) },
    )?;

    Ok(())
}

/// Accumulates a guest `(section, fuel)` pair into the current frame's tally.
fn register_profile_section_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_profile_section",
        |mut caller: Caller<'_, HostState>, name_ptr: u32, name_len: u32, fuel: u64| {
            let Some(name) = read_string(&caller, name_ptr, name_len) else {
                return;
            };
            *caller.data_mut().profile_sections.entry(name).or_default() += fuel;
        },
    )?;

    Ok(())
}

fn register_timezone_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_tz_convert",
        |mut caller: Caller<'_, HostState>,
         unix_secs: i64,
         tz_ptr: u32,
         tz_len: u32,
         out_ptr: u32|
         -> i32 {
            let Some(tz_name) = read_string(&caller, tz_ptr, tz_len) else {
                return -1;
            };

            let Some(buf) = tz_convert_impl(unix_secs, &tz_name) else {
                return -1;
            };

            let memory = caller.get_export("memory").and_then(Extern::into_memory);
            if let Some(memory) = memory {
                let data = memory.data_mut(&mut caller);
                let start = out_ptr as usize;
                if start + 20 <= data.len() {
                    data[start..start + 20].copy_from_slice(&buf);
                }
            }
            0
        },
    )?;

    Ok(())
}

fn xml_lookup_text(
    xml_docs: &std::collections::HashMap<XmlId, XmlDocumentIndex>,
    doc_id: XmlId,
    path: &str,
) -> Option<String> {
    xml_docs.get(&doc_id)?.get_str(path).map(str::to_owned)
}

fn validate_kv_key(key: &str) -> Result<()> {
    if key.is_empty() || key.contains('/') || key.contains('\\') || key.contains("..") {
        bail!("invalid KV key");
    }
    Ok(())
}

fn kv_disk_path(base: &std::path::Path, key: &str) -> Result<std::path::PathBuf> {
    validate_kv_key(key)?;
    Ok(base.join(key))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use bmc_wasm_protocol::XmlId;

    use crate::xml::XmlDocumentIndex;

    use super::{kv_disk_path, validate_kv_key, xml_lookup_text};

    const XML_WIDGET_FEED: &str = r#"
        <rss>
            <channel>
                <item>
                    <title>Launch</title>
                    <pubDate>Sat, 12 Apr 2026 18:00:00 GMT</pubDate>
                    <ttl>15</ttl>
                    <res duration="00:01:02" />
                </item>
            </channel>
        </rss>
    "#;

    #[test]
    fn kv_key_validation_rejects_path_traversal_sequences() {
        assert!(validate_kv_key("../secret").is_err());
        assert!(validate_kv_key("subdir/key").is_err());
        assert!(validate_kv_key(r"subdir\key").is_err());
        assert!(validate_kv_key("").is_err());
        assert!(validate_kv_key("plain_key").is_ok());
    }

    #[test]
    fn kv_path_for_valid_key_stays_under_base_dir() {
        let base = Path::new("/tmp/widget-kv");
        let path = kv_disk_path(base, "pairing_guid").expect("BUG: valid key should resolve");

        assert_eq!(path, base.join("pairing_guid"));
    }

    #[test]
    fn xml_lookup_reads_multiple_fields_from_one_indexed_document() {
        let mut xml_docs = HashMap::new();
        let doc_id = XmlId::from_wire(1).expect("BUG: 1 is a valid wire ID");
        xml_docs.insert(
            doc_id,
            XmlDocumentIndex::from_xml(XML_WIDGET_FEED)
                .expect("BUG: test XML should build an index"),
        );

        assert_eq!(
            xml_lookup_text(&xml_docs, doc_id, "//title"),
            Some("Launch".to_owned())
        );
        assert_eq!(
            xml_lookup_text(&xml_docs, doc_id, "//pubDate"),
            Some("Sat, 12 Apr 2026 18:00:00 GMT".to_owned())
        );
        assert_eq!(
            xml_lookup_text(&xml_docs, doc_id, "//ttl").and_then(|value| value.parse::<f64>().ok()),
            Some(15.0)
        );
        assert_eq!(
            xml_lookup_text(&xml_docs, doc_id, "//res/@duration"),
            Some("00:01:02".to_owned())
        );
        assert_eq!(
            xml_lookup_text(&xml_docs, doc_id, "//title"),
            Some("Launch".to_owned())
        );

        xml_docs.remove(&doc_id);

        assert_eq!(xml_lookup_text(&xml_docs, doc_id, "//title"), None);
    }

    #[test]
    fn xml_lookup_f64_rejects_non_numeric_fields() {
        let mut xml_docs = HashMap::new();
        let doc_id = XmlId::from_wire(1).expect("BUG: 1 is a valid wire ID");
        xml_docs.insert(
            doc_id,
            XmlDocumentIndex::from_xml(XML_WIDGET_FEED)
                .expect("BUG: test XML should build an index"),
        );

        assert_eq!(
            xml_lookup_text(&xml_docs, doc_id, "//title")
                .and_then(|value| value.parse::<f64>().ok()),
            None
        );
    }
}
