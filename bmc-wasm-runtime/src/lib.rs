// Copyright (C) 2026  Braiins Systems s.r.o.

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
//! - [`WasmWidgetRuntime::wants_next_frame`] — the widget requested another
//!   frame (animation is active). Render again on the next vsync tick.
//! - [`WasmWidgetRuntime::next_frame_delay`] — the widget requested a delayed
//!   frame (`request_frame_after(ms)`). Schedule a wake-up after the delay
//!   instead of rendering immediately. **Do not busy-wait** for the delay.
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
//! - **Ignoring `next_frame_delay`** — treating delayed frames as immediate
//!   requests defeats the widget's own power management.
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

mod animation;
pub mod gpu;
pub mod renderer;

// Re-export colors and color macro from protocol crate
pub mod colors {
    pub use bmc_wasm_protocol::colors::*;
}
pub use bmc_wasm_protocol::color;
mod host_api;
#[cfg(feature = "perf-overlay")]
pub mod perf_overlay;
mod runtime;
pub mod tree;

pub mod components;
pub mod interaction;

#[cfg(feature = "fixtures")]
pub mod capture_config;
#[cfg(feature = "fixtures")]
pub mod fixtures;
#[cfg(feature = "fixtures")]
pub mod unified_fixture;

pub use host_api::{FixtureEvent, FixtureEventKind, FixtureEventState, FrameTimings};
pub use runtime::{RenderStatus, RuntimeConfig, WasmWidgetRuntime};
