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

//! WebAssembly runtime for remote widget overlays.
//!
//! This crate provides a sandboxed WASM execution environment for remote widgets
//! to render interactive overlays on top of server-rendered images.
//!
//! See `docs/plan.md` for the full design document.
//!
//! # Host integration guide
//!
//! A **host** is any application that embeds [`WasmWidgetRuntime`] to display
//! widgets — the on-device compositor, the desktop testbed, etc. The runtime
//! handles WASM execution, layout, and rendering; the host is responsible for
//! the event loop, GL context, and frame scheduling.
//!
//! ## Render loop — do not spin
//!
//! **Never run an uncapped render loop.** The runtime will happily consume 100%
//! CPU if you call [`WasmWidgetRuntime::render`] every iteration without
//! sleeping. Use vsync or an explicit frame timer to cap the rate.
//!
//! After each render call, check the scheduling hints:
//!
//! - [`WasmWidgetRuntime::wants_next_frame`] — another frame is needed, either
//!   because the widget asked for one or because cached-tree animations still
//!   need host-side updates.
//! - [`WasmWidgetRuntime::next_frame_delay`] — the next host wake delay.
//!   This may be the widget's original `request_frame_after(ms)` delay, or a
//!   shorter animation cadence while cached-tree animations are active.
//!   Schedule one wake-up after the returned delay instead of rendering
//!   immediately. **Do not busy-wait** for the delay.
//! - **Neither** — the widget is idle. Sleep until user input or an external
//!   event (e.g. data push) arrives. Do not poll.
//!
//! ```text
//! loop {
//!     renderer.begin_frame(w, h);
//!     runtime.render(delta_ms)?;
//!     renderer.flush();
//!     swap_buffers();  // vsync blocks here
//!
//!     if runtime.wants_next_frame() {
//!         if let Some(delay) = runtime.next_frame_delay() {
//!             sleep(delay);  // or schedule a timer
//!         }
//!         continue;
//!     }
//!     wait_for_input_or_event();  // idle — don't spin
//! }
//! ```
//!
//! ## Frame pipeline
//!
//! Each frame follows this sequence:
//!
//! 1. `renderer.begin_frame(w, h)` — set up the GL viewport
//! 2. `runtime.deliver_fetch_responses()` — deliver any completed HTTP fetches
//!    and `runtime.deliver_ws_messages()` — deliver any pending WebSocket events
//! 3. `runtime.render(delta_ms)` — execute WASM + layout + draw commands
//! 4. `renderer.flush()` — submit draw commands to the GPU
//! 5. Blit / swap buffers
//!
//! **Do not call `render()` if the widget doesn't need it.** Check
//! `wants_next_frame()` / `has_pending_fetches()` first. Skipping idle frames
//! saves both CPU (WASM interpreter) and GPU (draw calls + flush).
//!
//! ## What NOT to do
//!
//! These mistakes were found in the testbed and caused excessive CPU usage on
//! both desktop and the real device:
//!
//! - **Rendering unconditionally** — rendering every vsync even when the widget
//!   is idle wastes 100% of a core.
//! - **Ignoring `next_frame_delay`** — treating delayed wakes as immediate
//!   requests defeats both the widget's own pacing and the host scheduler.
//! - **Allocating per frame** — avoid creating fresh data structures (layout
//!   trees, text buffers) every frame; the runtime provides caching APIs.
//!
//! # Safety
//!
//! All remaining `unsafe` in the runtime and SDK is forced by external APIs:
//!
//! - **OpenGL** — glow/glutin require unsafe for every GL call, context creation,
//!   and function-pointer loading. FemtoVG's `OpenGl::new_from_function` inherits this.
//! - **WASM host FFI** — `unsafe extern "C"` blocks declaring host imports and
//!   `#[unsafe(no_mangle)]` on WASM exports are the only way to cross the
//!   host↔guest boundary.
//! - **WASM allocator protocol** — `__alloc`/`__dealloc`, `__on_fetch_response`,
//!   and `__on_ws_event` use `Vec::from_raw_parts` to transfer ownership of
//!   host-allocated buffers.

mod runtime_limits;
mod xml;

mod audio_registry;
pub mod disk_cache;
pub use disk_cache::DiskCache;
mod host_api;
pub mod led_request;
pub mod network;
mod package_assets;
#[cfg(feature = "perf-overlay")]
pub mod perf_overlay;
mod renderer_assets;
mod runtime;
pub mod system;

pub use package_assets::{PackageAssetError, PackageAssetStore};

#[cfg(feature = "fixtures")]
pub mod capture_config;
#[cfg(feature = "fixtures")]
pub mod fixtures;
#[cfg(feature = "capture")]
pub mod stack_profile;
#[cfg(feature = "fixtures")]
pub mod unified_fixture;

pub use bmc_led::data::{LedEffectKind as LedEffect, LedScope, Rgb};
pub use host_api::{FixtureEvent, FixtureEventKind, FixtureEventState};
pub use led_request::{LED_REQUEST_ID_ALL, LedRequest, LedRequestId, LedRequestIdAllocator};
pub use network::NetworkInfo;
pub use runtime::{
    BoundCredential, CredentialView, RenderStatus, RendererAssetRestorationObservation,
    RendererAssetSuspensionObservation, RuntimeConfig, RuntimeDisplayInfo, RuntimeResourceLimits,
    WasmWidgetModule, WasmWidgetRuntime,
};
pub use system::{NextAlarm, SystemSettings, SystemSnapshot};

/// Errors produced by [`parse_params_json`] when an entry violates the wire-format contract.
///
/// Either variant means the *entire* update is rejected
///  — `parse_params_json` does not return a partial map.
/// Callers are expected to keep their previous snapshot on `Err`.
#[derive(Debug, thiserror::Error)]
pub enum ParseParamsError {
    /// Key did not satisfy [`bmc_widget_manifest::ParamKey::try_new`]
    /// — invalid characters or over-cap length.
    #[error("invalid param key {key:?}")]
    InvalidKey { key: String },

    /// Value was an array, object, non-finite number, or other shape
    /// that [`bmc_widget_manifest::ParamValue::try_from`] rejects.
    #[error("param {key:?} value not representable: {source}")]
    InvalidValue {
        key: String,
        #[source]
        source: bmc_widget_manifest::ParamValueConversionError,
    },
}

/// Parse the wayland `deck_widget_v1.credentials` map into the typed view.
///
/// Unlike [`parse_params_json`] a malformed entry is skipped
/// instead of rejecting the whole map: to the widget an unreadable slot
/// looks exactly like an unbound one, and dropping the rest
/// would blind slots that are perfectly resolvable.
#[must_use]
pub fn parse_credentials_json(
    object: &serde_json::Map<String, serde_json::Value>,
) -> CredentialView {
    let mut slots = std::collections::BTreeMap::new();
    for (slot, entry) in object {
        let field = |name: &str| entry.get(name).and_then(serde_json::Value::as_str);
        if let (Some(type_id), Some(account_name)) = (field("type"), field("account")) {
            slots.insert(
                slot.clone(),
                BoundCredential {
                    type_id: type_id.to_owned(),
                    account_name: account_name.to_owned(),
                },
            );
        } else {
            tracing::warn!(
                slot,
                "credential entry missing type/account — slot reads unbound"
            );
        }
    }

    CredentialView::new(slots)
}

/// Parse the wayland `deck_widget_v1.params` map into the typed `BTreeMap` shape
/// that [`WasmWidgetRuntime::set_params`] expects.
///
/// On the first invalid entry the entire map is rejected with [`ParseParamsError`]
/// — callers should keep the previous snapshot rather than apply a partial update
/// (a partial update with stale keys is worse than no update).
///
/// The compositor's validator stands upstream and is expected to filter manifests
/// before this path; a failure here therefore indicates an off-spec producer
/// (compositor bug) and is logged at `warn` level by callers.
///
/// Accepts only scalar shapes (string / integer / finite double / boolean / null);
/// Arrays, objects, and non-finite numbers reject.
///
/// Keys must satisfy the `ParamKey` regex (start with an ASCII letter,
/// then `[A-Za-z0-9_-]*`) and stay within the manifest-layer length cap.
pub fn parse_params_json(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<
    std::collections::BTreeMap<bmc_widget_manifest::ParamKey, bmc_widget_manifest::ParamValue>,
    ParseParamsError,
> {
    let mut out = std::collections::BTreeMap::new();
    for (raw_key, raw_value) in object {
        let key = bmc_widget_manifest::ParamKey::try_new(raw_key.clone())
            .map_err(|bad| ParseParamsError::InvalidKey { key: bad })?;
        let value = bmc_widget_manifest::ParamValue::try_from(raw_value).map_err(|source| {
            ParseParamsError::InvalidValue {
                key: raw_key.clone(),
                source,
            }
        })?;
        out.insert(key, value);
    }
    Ok(out)
}

/// Initial params snapshot from a manifest: every declared key
/// bound to its `default_value`. Mirrors what the compositor
/// delivers on-device when no operator overrides are set.
#[must_use]
pub fn manifest_default_params(
    manifest: &bmc_widget_manifest::Manifest,
) -> std::collections::BTreeMap<bmc_widget_manifest::ParamKey, bmc_widget_manifest::ParamValue> {
    manifest
        .params
        .iter()
        .map(|(key, def)| {
            (
                key.clone(),
                bmc_widget_manifest::ParamValue::from_param_kind_default(&def.kind),
            )
        })
        .collect()
}

#[cfg(test)]
mod credential_parse_tests {
    use super::*;

    fn object(json: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        json.as_object()
            .expect("BUG: json! literal is an object")
            .clone()
    }

    #[test]
    fn a_well_formed_entry_becomes_a_bound_slot() {
        let view = parse_credentials_json(&object(&serde_json::json!({
            "pool": { "type": "braiins-pool", "account": "My pool" }
        })));

        assert_eq!(view.slot_count(), 1);
    }

    #[test]
    fn a_malformed_entry_is_skipped_without_losing_its_neighbours() {
        let view = parse_credentials_json(&object(&serde_json::json!({
            "pool": { "type": "braiins-pool", "account": "My pool" },
            "broken": { "type": "generic-token" },
            "alsobroken": "not-an-object"
        })));

        assert_eq!(
            view.slot_count(),
            1,
            "one bad slot must not blind the slots that parse"
        );
    }
}
