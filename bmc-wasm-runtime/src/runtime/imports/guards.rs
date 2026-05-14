// Copyright (C) 2026  Braiins Systems s.r.o.

//! Lifecycle-phase guards for guest imports.
//!
//! Most host imports are only meaningful inside specific guest-call phases (`init` / `render` / `on_params_update` / `unload`).
//! The runtime tracks the active phase in [`HostState::current_lifecycle`].
//! This module checks the matrix at each import call site and surfaces violations as:
//!
//! * [`require_render`] — trap with a clear message naming the rule.
//!   Used for imports that can only produce a meaningful side effect during `render`
//!   (tree submission, touch readback), where a violation indicates a structural
//!   widget bug we want to surface loudly rather than paper over.
//! * [`forbid_unload`] — return without effect after `tracing::warn` once.
//!   Used for imports that are *meaningful* in most phases but pointless during
//!   teardown (frame requests after `unload` would be acted on by a runtime that's
//!   about to be dropped).

use std::sync::atomic::{AtomicBool, Ordering};

use wasmi::Caller;

use crate::host_api::{HostState, Lifecycle};

/// Trap the guest call with a clear message naming the rule that was violated.
///
/// Returns the trap as an `Err` so the caller can propagate it; this is intended to be
/// used at the very top of an import body with `?` (when the import returns `Result`)
/// or with an early return when the import returns a sentinel.
fn lifecycle_trap(
    import_name: &'static str,
    allowed: &'static str,
    actual: Lifecycle,
) -> wasmi::Error {
    wasmi::Error::new(format!(
        "host import `{import_name}` is only legal inside {allowed}, \
         but the guest called it from {actual:?} — see the lifecycle guard \
         matrix in `bmc_wasm_sdk` crate docs for the documented rules"
    ))
}

/// Require [`Lifecycle::Render`] for an import; trap otherwise.
///
/// Use for imports whose effects are only meaningful in a render frame
/// (`host_submit_tree`, touch readback). A violation is a widget bug — calling render-only
/// imports from `on_params_update` would, e.g., race the next real render or stash a stale
/// tree, so trapping is the only honest behaviour.
pub(super) fn require_render(
    caller: &Caller<'_, HostState>,
    import_name: &'static str,
) -> Result<(), wasmi::Error> {
    let actual = caller.data().current_lifecycle;
    if actual == Lifecycle::Render {
        Ok(())
    } else {
        Err(lifecycle_trap(import_name, "`render`", actual))
    }
}

/// Require [`Lifecycle::Render`] for an import; return `false` after a warn-once otherwise.
///
/// Use for imports whose effects are only meaningful in a render frame but where a violation
/// is reasonable to soft-fail rather than trap — touch readback (`host_get_touch_click`,
/// `host_get_touch_drag`) is the canonical case.
///
/// A widget that reads a touch outside `render` gets `None` back, the same shape it would see
/// for a no-touch frame, so callers compose naturally; trapping here would punish a widget
/// for reading defensively.
///
/// Returns `true` if the import body should run,
/// `false` if it should return its "nothing-here" sentinel (typically `0`).
pub(super) fn render_or_warn(
    caller: &Caller<'_, HostState>,
    import_name: &'static str,
    warned: &AtomicBool,
) -> bool {
    let actual = caller.data().current_lifecycle;
    if actual == Lifecycle::Render {
        return true;
    }
    if !warned.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            "host import `{import_name}` called outside `render` (was {actual:?}) — \
             returning empty. See the lifecycle guard matrix in `bmc_wasm_sdk` crate docs."
        );
    }
    false
}

/// Forbid [`Lifecycle::Unload`] for an import; return `false` after a warn-once otherwise.
///
/// Use for imports that are valid in most phases but pointless during teardown
/// (`host_request_frame{,_after}` after `unload` would queue work on a runtime
/// that's about to be dropped).
///
/// The runtime never traps here — a widget legitimately might `request_frame`
/// from a teardown path on a control flow it doesn't fully own, and trapping
/// would obscure the real failure.
///
/// Returns `true` if the call should proceed,
/// `false` if the runtime should silently no-op.
pub(super) fn forbid_unload(
    caller: &Caller<'_, HostState>,
    import_name: &'static str,
    warned: &AtomicBool,
) -> bool {
    if caller.data().current_lifecycle != Lifecycle::Unload {
        return true;
    }
    if !warned.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            "host import `{import_name}` called during `unload` — ignored. \
             Unload is a synchronous teardown phase; frame requests cannot be honoured."
        );
    }
    false
}

/// Convenience for the warn-once latches used by [`forbid_unload`].
/// Declare one per import site so the warn fires at most once per process per import,
/// not once per widget instance, which would still be a flood for hot-reload churn.
pub(super) const fn warned_latch() -> AtomicBool {
    AtomicBool::new(false)
}
