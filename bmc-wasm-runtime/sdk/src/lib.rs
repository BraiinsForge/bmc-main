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

//! WASM Widget SDK for Braiins Deck.
//!
//! Widgets compile to wasm32 and render host-side (Taffy layout, FemtoVG GL,
//! cosmic-text shaping). The SDK is mostly tree-building primitives plus
//! thin shims around host-imported functions — layout / animation / text shaping
//! live on the host so the wasm binaries stay small.
//!
//! When compiled for native targets (non-wasm32), FFI-dependent modules are gated out.
//! The tree-building API ([`col`], [`row`], [`fn@text`], [`button!`], [`props!`], [`style!`])
//! and pure types remain available for the gallery and other native consumers.
//!
//! # Widget lifecycle
//!
//! A widget process spends its life in one of four lifecycle phases.
//! The host tracks the active phase and uses it to gate host imports
//! — some trap when called from the wrong phase, others soft-fail
//! (see the [Lifecycle guard matrix](#lifecycle-guard-matrix) below).
//!
//! ```text
//! process start
//!   └─ host instantiates the wasm module
//!        └─ (optional) calls `init`              ── one-shot setup
//!             ↓
//!        ┌──── enters the per-frame loop ───────┐
//!        │      ↓                               │
//!        │  calls `render(delta_ms)`            │── once per visible frame
//!        │      ↓                               │
//!        │  may fire `on_params_update`         │── after each operator-driven
//!        │      (when params version bumps)     │   per-widget `params` change
//!        │      ↓                               │
//!        │  may fire `on_system_update`         │── after each deck-wide
//!        │      (when system version bumps)     │   `system` snapshot change
//!        │      ↓                               │
//!        │  may fire `on_credentials_update`    │── after each credential
//!        │      (when its version bumps)        │   binding or secret change
//!        │      ↓                               │
//!        │  may fire `unload`                   │── when the widget is being torn
//!        │      (terminal — runs once)          │   down (scene swap, hot reload)
//!        └──────────────────────────────────────┘
//! ```
//!
//! `on_params_update`, `on_system_update`, and `on_credentials_update` are
//! independent exports — each fires only for its own channel's delivery. A
//! widget can export any combination of them, or none.
//!
//! Delivery updates the snapshot and invokes the optional hook, but does not
//! itself schedule a render. Call `request_frame()` or `request_frame_after()`
//! when the change should repaint; otherwise the new snapshot is observed on
//! the next naturally scheduled render.
//!
//! The host always invokes hooks one at a time on the wasm thread — there
//! is no concurrent execution inside a widget.
//!
//! ## Per-hook reference
//!
//! ### `init`
//!
//! ```rust,ignore
//! #[unsafe(no_mangle)]
//! pub extern "C" fn init() { /* one-shot setup */ }
//! ```
//!
//! Optional. The host runs it once during instantiation if exported.
//! Use for setup that must complete before the first `render`: KV reads to
//! restore persisted state, kick-off fetches whose response will populate the
//! first frame. (The SDK auto-installs the panic hook — no widget setup needed.)
//!
//! Viewport dimensions are no longer passed as arguments — call [`widget_size`]
//! from anywhere (in `init`, in `render`, in helpers) and you get the same `WidgetSize`
//! on every call. Most widgets don't need a thread-local copy at all.
//!
//! The initial params snapshot is already staged by the time `init` runs
//! — `params::current()` (or the typed accessor emitted by `bmc-widget-codegen`)
//! returns operator-configured values, not an empty map.
//!
//! ### `render(delta_ms)`
//!
//! ```rust,ignore
//! #[unsafe(no_mangle)]
//! pub extern "C" fn render(delta_ms: u32) {
//!     let WidgetSize { width: w, height: h, .. } = widget_size();
//!     render_ui(w, h, build_tree(/* … */));
//! }
//! ```
//!
//! The hot path. Called once per visible frame with the wall-clock delta
//! since the previous `render`. The widget builds a tree, calls `render_ui`
//! to submit it, and returns.
//!
//! `delta_ms` is intended for per-frame timer / animation bookkeeping (metronome elapsed-ms,
//! pomodoro countdown). Animations declared on the tree (`Draw::animate`, transitions)
//! are evolved host-side and don't need wasm-side tick state.
//!
//! ### `on_params_update`
//!
//! ```rust,ignore
//! #[unsafe(no_mangle)]
//! pub extern "C" fn on_params_update() {
//!     let Some(previous) = Params::previous() else { return };
//!     let changed = Params::current().changed_keys(&previous);
//!     // …
//! }
//! ```
//!
//! Optional — fires for every per-widget [`params`] snapshot delivery
//! *after* the initial values staged for `init`. Inside the hook,
//! [`params::previous`] holds the just-replaced snapshot — diff against
//! [`params::current`] to react only to keys whose value actually
//! changed. See [`params`] for the byte-level protocol and caching.
//!
//! Channel isolation: this hook fires only on params deliveries.
//! The deck-wide [`system`] channel has its own [`on_system_update`](#on_system_update)
//! hook so each diff sees a fresh, just-rotated `previous()` — a unified hook
//! would re-fire on the other channel's deliveries and surface its stale rotation
//! as a spurious diff.
//!
//! The initial delivery (staged before `init` runs) does NOT fire
//! this hook — only operator-driven mid-life changes do.
//!
//! ### `on_system_update`
//!
//! ```rust,ignore
//! #[unsafe(no_mangle)]
//! pub extern "C" fn on_system_update() {
//!     let cur = system::current();
//!     let prev = system::previous();
//!     // diff cur vs prev field-by-field
//! }
//! ```
//!
//! Optional — fires for every deck-wide [`system`] snapshot delivery
//! *after* the initial values staged for `init`. Sibling of
//! `on_params_update` for the system channel; same lifecycle and import
//! semantics, same "initial delivery doesn't fire" rule.
//!
//! Use this when a widget renders something that depends on deck-wide
//! state (timezone, formats, next-alarm, night-mode, …) and needs to
//! react to mid-life changes rather than just re-reading
//! [`system::current`] on every `render`.
//!
//! ### `on_credentials_update`
//!
//! ```rust,ignore
//! #[unsafe(no_mangle)]
//! pub extern "C" fn on_credentials_update() {
//!     let current = credentials::current();
//!     let previous = credentials::previous();
//!     if current != previous {
//!         request_frame();
//!     }
//! }
//! ```
//!
//! Optional — fires after each public binding or secret credential delivery.
//! The guest can inspect only the public binding view through
//! [`credentials::current`] and [`credentials::previous`]; secret rotation
//! still fires the hook without exposing the new value. Request a frame only
//! when the changed public view affects visible output.
//!
//! ### `on_touch`
//!
//! ```rust,ignore
//! #[unsafe(no_mangle)]
//! pub extern "C" fn on_touch() {
//!     request_frame();
//! }
//! ```
//!
//! Optional — fires once per host input drain in which the widget's surface
//! received any touch activity (down / move / up / cancel), coalesced like the
//! update hooks. It carries no arguments: it is the notification that a touch
//! happened, not the touch itself. The touch is consumed at the next `render`
//! through the usual readback (`Touch::click` / `Touch::drag`, button clicks,
//! scroll offsets) — `on_touch`'s job is only to decide whether that render
//! should happen, by calling `request_frame` (or `request_frame_after`).
//!
//! A widget that consumes touch **must** export this hook. The host does not
//! render on touch by itself; without `on_touch` (and a frame request from it)
//! a touch produces no render, so buttons, sliders, and scroll views stay
//! inert. A purely passive widget that ignores touch simply omits it.
//!
//! ### `on_network_update`
//!
//! ```rust,ignore
//! #[unsafe(no_mangle)]
//! pub extern "C" fn on_network_update() {
//!     if network_is_on_screen() {
//!         request_frame();
//!     }
//! }
//! ```
//!
//! Optional — fires when the Deck's own SSID or IP changed
//! (signal-strength jitter never fires it).
//! Like `on_touch` it carries no arguments:
//! the widget re-reads [`network::info`], decides whether the change is visible
//! on its current screen, and requests a frame only then.
//! Unlike `on_touch`, it can fire in any lifecycle state — including on a
//! dormant widget, before its first `on_wake` — so it must not rely on state
//! that `on_sleep` tears down.
//! The host never renders on network changes by itself; a widget that displays
//! the Deck's SSID/IP and omits this hook shows stale values until its next
//! natural render.
//!
//! ### `unload`
//!
//! ```rust,ignore
//! #[unsafe(no_mangle)]
//! pub extern "C" fn unload() { /* persist state, flush buffers */ }
//! ```
//!
//! Optional. The host runs it once, synchronously, immediately before the widget instance
//! is dropped (scene swap, hot reload, shutdown). The place to flush in-memory state to KV.
//! Frame requests fired from `unload` are silently ignored — the runtime is about to be torn down.
//!
//! ### `on_wake` / `on_sleep`
//!
//! ```rust,ignore
//! #[unsafe(no_mangle)]
//! pub extern "C" fn on_sleep() { /* release off-scene resources */ }
//! #[unsafe(no_mangle)]
//! pub extern "C" fn on_wake() { /* rebuild guest state before the first frame */ }
//! ```
//!
//! Optional, paired with the dormancy edge. `on_sleep` fires when the widget
//! scrolls off-scene; `on_wake` fires before the first frame when it returns.
//! After `on_sleep`, the host releases package- and cache-backed renderer payloads
//! while preserving their IDs. `on_wake` runs without restoring them. The host
//! restores a package/cache ID at its first draw,
//! whether the tree is newly submitted or cached.
//! Layout does not pre-load other assets.
//! A missing cache entry remains recoverable through the widget's normal refetch path.
//! Assets registered from guest memory have no external restore source, so they stay
//! resident and usable while dormant for compatibility with SDK 0.2 widgets.
//! Explicit `Slot::evict()` or `evict_all()` always destroys the reservation,
//! regardless of lifecycle phase.
//! Sleep and wake can coalesce before renderer delivery.
//! In that case, both hooks run without suspending already-resident assets.
//! Package/cache reservations created by `on_sleep` remain suspended until
//! a draw uses them. `on_sleep` remains the place to persist guest state needed
//! after wake.
//!
//! Dormancy stops renders, not deliveries: param, system and credential
//! updates, fetch replies and socket callbacks all keep arriving off-scene.
//! A widget that starts work from them therefore keeps the host busy for a
//! scene nobody is looking at. There is no import that answers "am I dormant";
//! a widget that needs to know tracks the `on_sleep`/`on_wake` edge itself.
//!
//! ## Lifecycle guard matrix
//!
//! A handful of host imports are only meaningful inside specific hooks.
//! The runtime enforces this and surfaces violations in one of two ways:
//!
//! - **trap** — the import returns a wasmi trap, which propagates out
//!   of the offending guest call and kills the widget.
//!   Used when calling the import from the wrong phase indicates a structural
//!   widget bug whose silent no-op would mask a real failure.
//! - **soft-fail** — the import logs a warn-once and returns a "nothing here" sentinel
//!   (`None` for touch reads, no-op for frame requests). Used when reading defensively
//!   is reasonable and the widget composes naturally with the sentinel.
//!
//! `on_params_update`, `on_system_update`, `on_credentials_update`,
//! `on_touch`, `on_network_update`, `on_wake`, and `on_sleep` share the
//! same import-legality row — state-mutation legal, tree-submission illegal.
//!
//! | Import                                              | `init` | `render` | `on_*`* | `unload` |
//! |-----------------------------------------------------|:------:|:--------:|:-------:|:--------:|
//! | `render_ui` / `host_submit_tree`                    | trap   | ✓        | trap    | trap     |
//! | `Touch::click` / `Touch::drag` (touch readback)     | None¹  | ✓        | None¹   | None¹    |
//! | `request_frame` / `request_frame_after`             | ✓      | ✓        | ✓       | no-op²   |
//! | All other imports (params, KV, fetch, log, …)       | ✓      | ✓        | ✓       | ✓        |
//!
//! \* Same gating for `on_params_update`, `on_system_update`,
//!   `on_credentials_update`, `on_touch`, and `on_network_update`.
//!
//! ¹ Returns the touch-not-present sentinel after a one-time warn.
//!   Defensive reads compose naturally — the widget gets `None`
//!   the same way it would on a frame where no touch occurred.
//!
//! ² Silently dropped after a one-time warn. Honouring the request
//!   would queue work on a runtime that's about to be dropped.

// cast_sign_loss only fires on wasm32 (gated FFI code).
#![cfg_attr(target_arch = "wasm32", expect(clippy::cast_sign_loss))]

// SDK instantiation handshake the host calls once: installs the panic hook,
// returns the SDK version for the compat check.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn __bmc_sdk_init() -> u64 {
    install_panic_hook();
    version_pack(SDK_VERSION)
}

/// Logs widget panics as `file:line: message` — file name only, since the build
/// path is meaningless for out-of-tree widgets and the host adds the widget name.
/// Without it, `panic = "abort"` traps as a bare `unreachable`.
///
/// Verified on-device — a missing-param panic that was previously an opaque
/// `unreachable` trap now reads in the host log (widget name supplied by the
/// host's per-widget tracing span):
///
/// ```text
/// ERROR widget{wasm="weather.wasm"}: widget panic at typed.rs:52: BUG: required param `location` missing from snapshot
/// ```
#[cfg(target_arch = "wasm32")]
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let (file, line) = info.location().map_or(("?", 0), |l| {
            (
                l.file().rsplit(['/', '\\']).next().unwrap_or(l.file()),
                l.line(),
            )
        });
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("panic");
        crate::log_error!("widget panic at {}:{}: {}", file, line, msg);
    }));
}

// -- WASM-only modules (require host FFI) --
#[cfg(target_arch = "wasm32")]
pub mod alloc;
pub mod assets;
mod availability;
pub mod cache;
pub mod calendar;
pub mod credentials;
pub mod format;
pub mod host;
#[cfg(target_arch = "wasm32")]
pub mod http_listener;
#[cfg(target_arch = "wasm32")]
pub mod json;
pub mod json_str;
#[cfg(target_arch = "wasm32")]
pub mod kv;
#[cfg(target_arch = "wasm32")]
pub mod led;
#[cfg(target_arch = "wasm32")]
pub mod log;
#[cfg(feature = "math-3d")]
pub mod math;
#[cfg(target_arch = "wasm32")]
pub mod mdns;
pub mod mesh;
pub mod modal;
#[cfg(any(target_arch = "wasm32", test))]
pub mod net;
pub mod network;
pub mod notification;
pub mod number_input;
pub mod orientation;
pub mod params;
pub mod poll;
pub mod profile;
pub mod progress_bar;
pub mod relative_time;
pub mod skeleton;
#[cfg(target_arch = "wasm32")]
pub mod slot;
pub mod status_overlay;
pub mod switcher;
pub mod system;
pub mod tag;
// Snapshot-cache machinery is wasm32-only (consumed by the wasm-target public API)
// plus pulled in under `cfg(test)` for the unit tests that exercise the generic with a mock host.
// Native non-test builds don't use it — gate it so the dead-code lint stays happy.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) mod snapshot_cache;
#[cfg(target_arch = "wasm32")]
pub mod socket;
#[cfg(target_arch = "wasm32")]
pub mod ssdp;
pub mod text;
pub mod tree;
pub mod tz;
#[cfg(target_arch = "wasm32")]
pub mod udp_broadcast;
pub mod url;
// Private module, types re-exported at the crate root below — so `units` can't
// clash with the widget-local `units` crate some widgets still depend on.
mod units;
#[cfg(target_arch = "wasm32")]
pub mod ws;
#[cfg(target_arch = "wasm32")]
pub mod xml;

pub use availability::Availability;
pub use bmc_render_macros::*;
pub use bmc_wasm_protocol::*;
pub use bmc_wasm_sdk_macros::*;
#[cfg(target_arch = "wasm32")]
pub use calendar::{parse_calendar_date, parse_datetime};
#[cfg(target_arch = "wasm32")]
pub use format::{
    FormatDateOpts, FormatTimeOpts, format_date, format_duration, format_f64_fixed, format_time,
    local_unix_secs, resolve_tz_offset, strftime,
};
pub use host::*;
#[cfg(target_arch = "wasm32")]
pub use json::{JsonDoc, JsonKind};
pub use json_str::JsonStr;
#[cfg(target_arch = "wasm32")]
pub use led::LedEffect;
pub use mesh::*;
#[cfg(target_arch = "wasm32")]
pub use net::*;
pub use network::NetworkInfo;
pub use number_input::*;
pub use orientation::Orientation;
#[cfg(target_arch = "wasm32")]
pub use poll::{
    Config as PollConfig, FetchSpec, Handle as PollHandle, Method as FetchMethod,
    register as register_poll,
};
#[cfg(target_arch = "wasm32")]
pub use slot::*;
pub use tree::*;
pub use tz::Tz;
pub use ufmt;
pub use units::{
    BitcoinAmount, ElectricPower, Hashrate, Hashvalue, Length, Mass, MiningEfficiency, Ratio,
    Speed, Temperature,
};
#[cfg(target_arch = "wasm32")]
pub use ws::{Ws, WsEvent, ws_connect};
#[cfg(target_arch = "wasm32")]
pub use xml::XmlDoc;

/// Helper for `button!` macro — converts label to String.
#[doc(hidden)]
pub fn __macro_string_from(s: impl Into<String>) -> String {
    s.into()
}

// ── Props field coercion ─────────────────────────────────────────────

/// Trait that allows integer literals to be used for `f32` fields in `props!`.
///
/// Identity impl covers all types (f32→f32, bool→bool, Color→Color, etc.).
/// Additional impls convert integer types to f32 so you can write
/// `props!(gap: 12, padding: 16)` instead of `props!(gap: 12.0, padding: 16.0)`.
#[doc(hidden)]
pub trait PropsFieldValue<T> {
    fn into_field(self) -> T;
}

impl<T> PropsFieldValue<T> for T {
    fn into_field(self) -> T {
        self
    }
}

macro_rules! impl_int_to_f32_lossless {
    ($($t:ty),*) => {
        $(impl PropsFieldValue<f32> for $t {
            fn into_field(self) -> f32 { f32::from(self) }
        })*
    };
}

macro_rules! impl_int_to_f32_lossy {
    ($($t:ty),*) => {
        $(impl PropsFieldValue<f32> for $t {
            #[expect(clippy::cast_precision_loss)]
            fn into_field(self) -> f32 { self as f32 }
        })*
    };
}

impl_int_to_f32_lossless!(i8, i16, u8, u16);
impl_int_to_f32_lossy!(i32, i64, u32, u64, isize, usize);

/// Shorthand for PropsData: `props!()` or `props!(gap: 16, background: 0xFF)`
///
/// Integer literals are automatically converted to `f32` for layout fields
/// (padding, margin, gap, width, height, etc.).
///
/// Supports a special `bg_nine_patch: <NinePatch>` field that expands a
/// `NinePatch` into the underlying `bg_np_*` fields on `PropsData`.
#[macro_export]
macro_rules! props {
    () => { $crate::tree::PropsData::default() };
    ($($field:ident: $value:expr),* $(,)?) => {{
        let mut p = $crate::tree::PropsData::default();
        $(props!(@set p, $field: $value);)*
        p
    }};
    (@set $p:ident, bg_nine_patch: $v:expr) => {{
        let np = $v;
        $p.bg_np_id = np.bitmap_id;
        $p.bg_np_left = np.left;
        $p.bg_np_top = np.top;
        $p.bg_np_right = np.right;
        $p.bg_np_bottom = np.bottom;
    }};
    (@set $p:ident, $field:ident: $v:expr) => {
        $p.$field = $crate::PropsFieldValue::into_field($v);
    };
}

/// Button node with keyword-style options and sensible defaults.
///
/// First positional arg is the click ID (string), second is the label.
/// Style and size accept bare variant names resolved through the SDK.
///
/// # Examples
/// ```ignore
/// button!("ok", "OK")                                        // Primary, Normal, no icon
/// button!("delete", "Delete", style: Danger)                 // Danger, Normal
/// button!("reset", "Reset", style: Secondary, size: Small)   // Secondary, Small
/// button!("search", "", icon: gear_id)                       // icon-only, Primary, Normal
/// ```
#[macro_export]
macro_rules! button {
    // Accumulator pattern: parse fields one at a time, then emit.
    // Entry point: id, label, then optional keyword args.
    ($id:expr, $label:expr $(, $($rest:tt)*)?) => {
        button!(@acc
            [$id]
            [$label]
            [style: $crate::ButtonStyle::Primary]
            [size: $crate::ButtonSize::Normal]
            [icon: None]
            [disabled: false]
            [skin: None]
            $($($rest)*)?
        )
    };

    // Terminal — all fields consumed, build the node.
    // `icon:` accepts either `SvgId` or `Option<SvgId>` (std provides
    // `From<T> for Option<T>`, so `.into()` covers both).
    (@acc [$id:expr] [$label:expr] [style: $s:expr] [size: $sz:expr] [icon: $i:expr] [disabled: $d:expr] [skin: $sk:expr] $(,)?) => {
        $crate::make_button($crate::__macro_string_from($id), $crate::__macro_string_from($label), $s, $sz, $i.into(), $d, $sk)
    };

    // style: Variant
    (@acc [$id:expr] [$label:expr] [style: $_s:expr] [size: $sz:expr] [icon: $i:expr] [disabled: $d:expr] [skin: $sk:expr]
     style: $v:ident $(, $($rest:tt)*)?) => {
        button!(@acc
            [$id]
            [$label]
            [style: $crate::ButtonStyle::$v]
            [size: $sz]
            [icon: $i]
            [disabled: $d]
            [skin: $sk]
            $($($rest)*)?
        )
    };

    // size: Variant
    (@acc [$id:expr] [$label:expr] [style: $s:expr] [size: $_sz:expr] [icon: $i:expr] [disabled: $d:expr] [skin: $sk:expr]
     size: $v:ident $(, $($rest:tt)*)?) => {
        button!(@acc
            [$id]
            [$label]
            [style: $s]
            [size: $crate::ButtonSize::$v]
            [icon: $i]
            [disabled: $d]
            [skin: $sk]
            $($($rest)*)?
        )
    };

    // icon: expr
    (@acc [$id:expr] [$label:expr] [style: $s:expr] [size: $sz:expr] [icon: $_i:expr] [disabled: $d:expr] [skin: $sk:expr]
     icon: $v:expr $(, $($rest:tt)*)?) => {
        button!(@acc
            [$id]
            [$label]
            [style: $s]
            [size: $sz]
            [icon: $v]
            [disabled: $d]
            [skin: $sk]
            $($($rest)*)?
        )
    };

    // disabled: expr
    (@acc [$id:expr] [$label:expr] [style: $s:expr] [size: $sz:expr] [icon: $i:expr] [disabled: $_d:expr] [skin: $sk:expr]
     disabled: $v:expr $(, $($rest:tt)*)?) => {
        button!(@acc
            [$id]
            [$label]
            [style: $s]
            [size: $sz]
            [icon: $i]
            [disabled: $v]
            [skin: $sk]
            $($($rest)*)?
        )
    };

    // skin: expr
    (@acc [$id:expr] [$label:expr] [style: $s:expr] [size: $sz:expr] [icon: $i:expr] [disabled: $d:expr] [skin: $_sk:expr]
     skin: $v:expr $(, $($rest:tt)*)?) => {
        button!(@acc
            [$id]
            [$label]
            [style: $s]
            [size: $sz]
            [icon: $i]
            [disabled: $d]
            [skin: $v]
            $($($rest)*)?
        )
    };
}

/// Progress bar node with keyword-style options and sensible defaults.
///
/// The first positional argument is the progress mode (required).
///
/// # Examples
/// ```ignore
/// progress_bar!(ProgressMode::Slider(0.5))
/// progress_bar!(ProgressMode::Indeterminate, active: true)
/// progress_bar!(ProgressMode::Slider(vol), touch_key: "volume", skin: slider_skin)
/// ```
#[macro_export]
macro_rules! progress_bar {
    // Entry point:
    ($mode:expr $(, $($rest:tt)*)?) => {
        progress_bar!(@acc
            [$mode]
            [touch_key: ""]
            [track_h: 2.0]
            [active: false]
            [fill_color: $crate::WHITE]
            [track_color: $crate::GRAY_60.with_alpha(0.44)]
            [bg_color: $crate::TRANSPARENT]
            [skin: None]
            $($($rest)*)?
        )
    };

    // Terminal — all fields consumed, build the node.
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $sk:expr] $(,)?) => {
        $crate::progress_bar($tk, $th, $mode, $a, $fc, $tc, $bg, $sk)
    };

    // touch_key: expr
    (@acc [$mode:expr] [touch_key: $_tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $sk:expr]
     touch_key: $v:expr $(, $($rest:tt)*)?) => {
        progress_bar!(@acc [$mode] [touch_key: $v] [track_h: $th] [active: $a]
            [fill_color: $fc] [track_color: $tc] [bg_color: $bg] [skin: $sk] $($($rest)*)?)
    };

    // track_h: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $_th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $sk:expr]
     track_h: $v:expr $(, $($rest:tt)*)?) => {
        progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $v] [active: $a]
            [fill_color: $fc] [track_color: $tc] [bg_color: $bg] [skin: $sk] $($($rest)*)?)
    };

    // active: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $_a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $sk:expr]
     active: $v:expr $(, $($rest:tt)*)?) => {
        progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $th] [active: $v]
            [fill_color: $fc] [track_color: $tc] [bg_color: $bg] [skin: $sk] $($($rest)*)?)
    };

    // fill_color: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $_fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $sk:expr]
     fill_color: $v:expr $(, $($rest:tt)*)?) => {
        progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $th] [active: $a]
            [fill_color: $v] [track_color: $tc] [bg_color: $bg] [skin: $sk] $($($rest)*)?)
    };

    // track_color: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $_tc:expr] [bg_color: $bg:expr] [skin: $sk:expr]
     track_color: $v:expr $(, $($rest:tt)*)?) => {
        progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $th] [active: $a]
            [fill_color: $fc] [track_color: $v] [bg_color: $bg] [skin: $sk] $($($rest)*)?)
    };

    // bg_color: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $_bg:expr] [skin: $sk:expr]
     bg_color: $v:expr $(, $($rest:tt)*)?) => {
        progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $th] [active: $a]
            [fill_color: $fc] [track_color: $tc] [bg_color: $v] [skin: $sk] $($($rest)*)?)
    };

    // skin: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $_sk:expr]
     skin: $v:expr $(, $($rest:tt)*)?) => {
        progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $th] [active: $a]
            [fill_color: $fc] [track_color: $tc] [bg_color: $bg] [skin: $v] $($($rest)*)?)
    };
}

/// Number input — key + value are required, rest via `NumberInputProps` struct.
///
/// # Examples
/// ```ignore
/// number_input!("work", 25, label: "Work", suffix: "min", min: 1, max: 60)
/// number_input!("count", n)  // minimal, all defaults
/// ```
#[macro_export]
macro_rules! number_input {
    ($key:expr, $value:expr $(, $($field:ident: $val:expr),* $(,)?)?) => {{
        #[allow(unused_mut)]
        let mut p = $crate::number_input::NumberInputProps::default();
        $($(p.$field = $val;)*)?
        $crate::number_input::number_input($key, ::core::convert::Into::into($value), &p)
    }};
}

/// Connect to a WebSocket with optional headers.
///
/// # Examples
/// ```ignore
/// ws!("ws://host/api", on_event)
/// ws!("ws://host/api", on_event, headers: [
///     ("Authorization", "Bearer xyz"),
///     ("X-Custom", "value"),
/// ])
/// ```
#[macro_export]
macro_rules! ws {
    ($url:expr, $callback:expr $(,)?) => {
        $crate::ws::ws_connect($url, None, $callback)
    };
    ($url:expr, $callback:expr, headers: [$(($k:expr, $v:expr)),+ $(,)?] $(,)?) => {{
        let joined = [$( concat!($k, ": ", $v) ),+].join("\n");
        $crate::ws::ws_connect($url, Some(&joined), $callback)
    }};
}

/// Lightweight string interpolation without pulling in `core::fmt`.
/// Drop-in replacement for `format!()` in widget code.
///
/// Supports captured variable syntax like `std::format!`.
/// Positional and captured args interleave by their position in the format string,
/// so `fmt!("{year}-{}", month)` prints `year-month` (not `month-year`).
/// Format specs are limited to what `ufmt` accepts (`{:?}`, `{:#?}`, `{:x}`,
/// `{:X}`, `{:b}`, `{:o}`); width / fill / precision are not supported,
/// except zero-pad widths on hex (`{:02x}`) — decimal `{:02}` is rejected.
#[macro_export]
macro_rules! fmt {
    ($($arg:tt)*) => {
        $crate::fmt_impl!(@ufmt_path = $crate::ufmt; $($arg)*)
    };
}

/// Unified style macro for text styling and layout.
///
/// Text style fields:
///  - size
///  - weight
///  - italic
///  - underline
///  - strikethrough
///  - line_height
///  - align
///  - max_width
///  - text_overflow (`TextOverflow::Clip`, `TextOverflow::Ellipsis`)
///  - outline_color, outline_width
///
/// Layout fields: padding, margin, gap, flex, width, height, background
/// Shared field: color (applies to both)
///
/// # Examples
/// ```ignore
/// text("Hello", style!(size: 24, color: WHITE, padding: 8.0))
/// paragraph(style!(size: 16, line_height: 1.3), [
///     span("Click ", ()),
///     span("Save", style!(weight: FontWeight::BOLD)),
///     span(" to confirm.", ()),
/// ])
/// ```
#[macro_export]
macro_rules! style {
    () => {
        $crate::tree::StyleResult($crate::tree::TextStyle::default(), $crate::tree::PropsData::default())
    };
    ($($field:ident: $value:expr),* $(,)?) => {{
        let mut ts = $crate::tree::TextStyle::default();
        let mut p = $crate::tree::PropsData::default();
        $(style!(@route ts, p, $field: $value);)*
        $crate::tree::StyleResult(ts, p)
    }};
    // Text style fields
    (@route $ts:expr, $p:expr, size: $v:expr) => { $ts.size = $v; };
    (@route $ts:expr, $p:expr, weight: $v:expr) => { $ts.weight = $v; };
    (@route $ts:expr, $p:expr, family: $v:expr) => { $ts.family = $v; };
    (@route $ts:expr, $p:expr, italic: $v:expr) => { $ts.italic = $v; };
    (@route $ts:expr, $p:expr, underline: $v:expr) => { $ts.underline = $v; };
    (@route $ts:expr, $p:expr, strikethrough: $v:expr) => { $ts.strikethrough = $v; };
    (@route $ts:expr, $p:expr, line_height: $v:expr) => { $ts.line_height = $v; };
    (@route $ts:expr, $p:expr, align: $v:expr) => { $ts.align = $v; };
    (@route $ts:expr, $p:expr, valign: $v:expr) => { $ts.vertical_align = $v; };
    (@route $ts:expr, $p:expr, text_overflow: $v:expr) => { $ts.text_overflow = $v; };
    (@route $ts:expr, $p:expr, max_width: $v:expr) => { $ts.max_width = $v; };
    (@route $ts:expr, $p:expr, outline_color: $v:expr) => { $ts.outline_color = $v; };
    (@route $ts:expr, $p:expr, outline_width: $v:expr) => { $ts.outline_width = $v; };
    // Layout fields (use coercion so integer literals work for f32 fields)
    (@route $ts:expr, $p:expr, padding: $v:expr) => { $p.padding = $crate::PropsFieldValue::into_field($v); };
    (@route $ts:expr, $p:expr, margin: $v:expr) => { $p.margin = $crate::PropsFieldValue::into_field($v); };
    (@route $ts:expr, $p:expr, gap: $v:expr) => { $p.gap = $crate::PropsFieldValue::into_field($v); };
    (@route $ts:expr, $p:expr, flex: $v:expr) => { $p.flex = $crate::PropsFieldValue::into_field($v); };
    (@route $ts:expr, $p:expr, width: $v:expr) => { $p.width = $crate::PropsFieldValue::into_field($v); };
    (@route $ts:expr, $p:expr, height: $v:expr) => { $p.height = $crate::PropsFieldValue::into_field($v); };
    (@route $ts:expr, $p:expr, background: $v:expr) => { $p.background = $v; };
    (@route $ts:expr, $p:expr, color: $v:expr) => { $ts.color = $v; };
}
