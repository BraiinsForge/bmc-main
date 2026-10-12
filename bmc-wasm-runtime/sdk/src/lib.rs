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
//! Viewport dimensions are not arguments — call [`widget_size`] from anywhere
//! (in `init`, in `render`, in helpers) and you get the same `WidgetSize`
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
//! changed. See [`params`] for the caching and `bmc_wasm_protocol::params`
//! for the byte-level protocol.
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
//! updates, fetch replies, socket callbacks and `timer` ticks
//! all keep arriving off-scene.
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
//! - **soft-fail** — the import warns once per widget (repeats log at debug level)
//!   and returns a "nothing here" sentinel
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
//!   Every delivery callback follows the same column,
//!   from fetch replies and socket events to `timer` ticks.
//!
//! ¹ Returns the touch-not-present sentinel after a warning, once per widget.
//!   Defensive reads compose naturally — the widget gets `None`
//!   the same way it would on a frame where no touch occurred.
//!
//! ² Dropped, with a warning once per widget. Honouring the request
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
/// Verified on-device — a widget panic reads in the host log
/// (widget name supplied by the host's per-widget tracing span):
///
/// ```text
/// ERROR widget{wasm="weather.wasm"}: widget panic at typed.rs:52: …
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

#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub mod __test_support;

#[cfg(target_arch = "wasm32")]
pub mod alloc;
pub mod assets;
pub mod cache;
pub mod calendar;
pub mod credentials;
pub mod format;
pub mod host;
pub mod http_listener;
pub mod json;
pub mod json_str;
pub mod kv;
pub mod led;
pub mod log;
#[cfg(feature = "math-3d")]
pub mod math;
pub mod mdns;
pub mod net;
pub mod network;
pub mod params;
pub mod poll;
pub mod profile;
pub mod slot;
pub mod switcher;
pub mod system;
// Snapshot-cache machinery is wasm32-only (consumed by the wasm-target public API)
// plus pulled in under `cfg(test)` for the unit tests that exercise the generic with a mock host.
// Native non-test builds don't use it — gate it so the dead-code lint stays happy.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) mod snapshot_cache;
pub mod socket;
pub mod ssdp;
#[cfg(any(target_arch = "wasm32", test))]
pub mod timer;
pub mod tree;
pub mod types;
pub mod tz;
pub mod udp_broadcast;
pub mod url;
pub mod ws;
pub mod xml;

pub use bmc_ui::*;
pub use bmc_wasm_protocol::*;
pub use bmc_wasm_sdk_macros::*;
pub use calendar::{parse_calendar_date, parse_datetime};
pub use format::{
    FormatDateOpts, FormatTimeOpts, format_date, format_duration, format_f64_fixed, format_time,
    local_unix_secs, resolve_tz_offset, strftime,
};
pub use host::*;
pub use json::{JsonDoc, JsonKind};
pub use json_str::JsonStr;
pub use led::LedEffect;
pub use net::*;
pub use network::NetworkInfo;
pub use poll::{
    Config as PollConfig, FetchSpec, Handle as PollHandle, Method as FetchMethod,
    register as register_poll,
};
pub use slot::*;
pub use tz::Tz;
pub use ufmt;
pub use ws::{Ws, WsEvent, ws_connect};
pub use xml::XmlDoc;

// `bmc_ui` and the protocol both have `ids` and `time` modules; the root presents
// the protocol's, which are the host contract. `skeleton` is the builders' module
// rather than the kind enum's, as widgets write `skeleton::text(…)`.
pub use bmc_ui::components::skeleton;
pub use bmc_wasm_protocol::{ids, time};

pub use assets::*;
pub use switcher::*;
pub use tree::*;

/// Connect to a WebSocket with optional headers, as `ws_connect` does.
///
/// It returns the same `Option<Ws>`. Dropping the `Ws` closes the connection, so keep it.
///
/// # Examples
/// ```ignore
/// WS.set(ws!("ws://host/api", on_event));
/// WS.set(ws!("ws://host/api", on_event, headers: [
///     ("Authorization", "Bearer xyz"),
///     ("X-Custom", "value"),
/// ]));
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

#[doc(hidden)]
pub mod __macros {
    pub use bmc_render_macros::*;
}

// The proc macros name the descriptor types through the root they are handed,
// and `$crate` is this crate whichever path a caller reached the wrapper by.

/// Compile an SVG into compact path data at build time, as a [`Svg`].
///
/// ```ignore
/// const STAR: Svg = include_svg!("assets/star.svg");
/// ```
///
/// The path is resolved relative to the crate's `CARGO_MANIFEST_DIR`,
/// walking up to parent directories for cross-crate scene files.
#[macro_export]
macro_rules! include_svg {
    ($($t:tt)*) => { $crate::__macros::include_svg!($crate, $($t)*) };
}

/// Package a raster image at build time, as a [`Bitmap`].
///
/// ```ignore
/// const FALCON_9: Bitmap = include_bitmap!("assets/falcon-9.png");
/// ```
#[macro_export]
macro_rules! include_bitmap {
    ($($t:tt)*) => { $crate::__macros::include_bitmap!($crate, $($t)*) };
}

/// Package an audio file (WAV, OGG, MP3) at build time, as an [`Audio`].
///
/// ```ignore
/// const TICK: Audio = include_audio!("assets/sounds/tick.wav");
/// ```
#[macro_export]
macro_rules! include_audio {
    ($($t:tt)*) => { $crate::__macros::include_audio!($crate, $($t)*) };
}

/// Pack a glTF 2.0 binary mesh at build time, as a [`Mesh`].
///
/// ```ignore
/// static SUZANNE: Mesh = include_mesh!("assets/suzanne.glb");
/// ```
#[macro_export]
macro_rules! include_mesh {
    ($($t:tt)*) => { $crate::__macros::include_mesh!($crate, $($t)*) };
}

/// Package a `.9.png` at build time, as a [`NinePatchAsset`] with its insets decoded.
///
/// ```ignore
/// const BUTTON_BG: NinePatchAsset = include_nine_patch!("assets/button.9.png");
/// ```
#[macro_export]
macro_rules! include_nine_patch {
    ($($t:tt)*) => { $crate::__macros::include_nine_patch!($crate, $($t)*) };
}

/// Embed a skin directory or zip at build time, as a [`Skin`] with named assets.
///
/// ```ignore
/// static MY_SKIN: Skin = include_skin!("assets/my-skin/");
/// ```
#[macro_export]
macro_rules! include_skin {
    ($($t:tt)*) => { $crate::__macros::include_skin!($crate, $($t)*) };
}
