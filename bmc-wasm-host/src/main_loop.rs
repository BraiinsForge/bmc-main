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

use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer;
use bmc_system_overlay::HostedOverlay;

use crate::cache_gc;
use crate::control::{ListenSocket, accept_and_load};
use crate::host::SharedHost;
use crate::slot::WidgetSlot;

/// Let a startup/scene burst settle before the next GC, per review.
const GC_SETTLE_DELAY: Duration = Duration::from_secs(5);

/// Exit as soon as the last connection is gone: bmc shuts down and restarts
/// immediately, and a host that lingers past its last slot would adopt the
/// new bmc instance's thins (stale code after a package upgrade) or hold the
/// socket against the replacement host's bind.
#[derive(Debug)]
pub struct HostLifetime {
    ever_had_slot: bool,
}

impl HostLifetime {
    #[must_use]
    pub fn new() -> Self {
        Self {
            ever_had_slot: false,
        }
    }

    /// Record the outcome of draining one accept burst: `loaded` slots
    /// inserted, `rejected` connections that failed to load, and `slots_len`
    /// slots alive afterwards.
    ///
    /// A successful load, or a rejection that leaves no surviving slot (a lone
    /// bad bootstrap widget), flips `ever_had_slot` so the host exits once idle
    /// instead of lingering. A rejection among healthy siblings is ignored: the
    /// siblings keep the host alive on their own and one bad widget must not
    /// tear it down.
    pub fn note_accept_burst(&mut self, loaded: usize, rejected: usize, slots_len: usize) {
        if loaded > 0 || (rejected > 0 && slots_len == 0) {
            self.ever_had_slot = true;
        }
    }

    #[must_use]
    pub fn should_continue(&self, slots_len: usize, overlays_active: bool) -> bool {
        slots_len > 0 || overlays_active || !self.ever_had_slot
    }
}

/// Pure-function inputs that `compute_poll_timeout_from_inputs` consumes per slot.
///
/// Extracted so the timeout policy can be unit-tested in isolation without standing up
/// a `WidgetSlot` (which needs a wasm runtime). The thin `compute_poll_timeout` wrapper
/// below collects these from each slot in production.
///
/// `min_inter_frame_remaining` collapses the production code's two-call pattern
/// (`has_min_inter_frame_elapsed` then `min_inter_frame_remaining`) into a single
/// field: `None` means the inter-frame floor has already elapsed (poll may return 0
/// immediately); `Some(Duration::ZERO)` is treated the same way; any other `Some(d)`
/// is the time remaining before the slot may render again.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is a distinct widget-slot predicate; a flags enum would be less readable at the call sites"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SlotPollInputs {
    pub retry_in: Option<Duration>,
    pub is_renderable: bool,
    pub is_blocked: bool,
    pub frame_callback_enabled: bool,
    pub animation_wants_immediate: bool,
    pub surface_needs_render: bool,
    pub min_inter_frame_remaining: Option<Duration>,
    pub next_frame_delay: Option<u32>,
    pub has_pending_io: bool,
}

/// Pure: fold per-slot inputs into a single `poll(2)` timeout in milliseconds.
/// `-1` is `poll(2)`'s indefinite-block sentinel, returned when nothing
/// contributes a finite value.
#[must_use]
pub fn compute_poll_timeout_from_inputs(slots: &[SlotPollInputs]) -> i32 {
    let mut best: Option<Duration> = None;
    let push = |best: &mut Option<Duration>, candidate: Duration| {
        *best = match *best {
            None => Some(candidate),
            Some(prev) => Some(prev.min(candidate)),
        };
    };

    for slot in slots {
        // `retry_in` drives lifecycle progression (e.g. Entering→Visible), not rendering,
        // so it contributes regardless of whether the slot wants to render right now.
        if let Some(d) = slot.retry_in {
            push(&mut best, d);
        }
        if !slot.is_renderable || slot.is_blocked {
            continue;
        }

        // Runtime-driven wakeups only contribute while Visible.
        // The surface flag arrives already masked by the off-screen dirty gate,
        // so a held-back update cannot wake the host for a render `needs_render` refuses.
        let animation_active = slot.frame_callback_enabled && slot.animation_wants_immediate;
        if slot.surface_needs_render || animation_active {
            match slot.min_inter_frame_remaining {
                None => return 0,
                Some(d) if d.is_zero() => return 0,
                Some(d) => push(&mut best, d),
            }
        } else if slot.frame_callback_enabled
            && let Some(d) = slot.next_frame_delay
        {
            // `Some(0)` is unreachable on this branch: animation_wants_immediate is
            // exactly `next_frame_delay == Some(0)`, which the branch above handles.
            push(&mut best, Duration::from_millis(d.into()));
        }
    }
    if slots.iter().any(|s| s.has_pending_io) {
        push(&mut best, Duration::from_millis(100));
    }
    match best {
        None => -1,
        Some(d) => i32::try_from(d.as_millis()).unwrap_or(i32::MAX),
    }
}

/// Production wrapper: gather every slot's inputs and forward to the pure core.
#[must_use]
pub fn compute_poll_timeout(slots: &SlotTable, now: Instant) -> i32 {
    let inputs: Vec<SlotPollInputs> = slots.iter().map(|s| s.poll_inputs(now)).collect();
    compute_poll_timeout_from_inputs(&inputs)
}

#[derive(Debug)]
pub enum FatalError {
    PollFailed(std::io::Error),
    ListenerLost(i16),
    AcceptFailed(std::io::Error),
    ControlSocketBindFailed(std::io::Error),
    EglContextLost,
}

#[derive(Debug)]
pub enum PollDecision {
    Retry,
    Fatal(std::io::Error),
}

pub fn classify_listener_revents(revents: i16) -> Result<(), FatalError> {
    const FATAL: i16 = libc::POLLERR | libc::POLLNVAL | libc::POLLHUP;
    if (revents & FATAL) != 0 {
        Err(FatalError::ListenerLost(revents))
    } else {
        Ok(())
    }
}

#[must_use]
pub fn classify_poll_errno(err: &std::io::Error) -> PollDecision {
    match err.raw_os_error() {
        Some(libc::EINTR) => PollDecision::Retry,
        _ => PollDecision::Fatal(std::io::Error::from_raw_os_error(
            err.raw_os_error().unwrap_or(0),
        )),
    }
}

#[must_use]
fn compact_error_message(message: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in message.chars().take(max_chars) {
        out.push(ch);
    }
    out
}

#[must_use]
fn overlay_dispatch_error_kind(error: &anyhow::Error) -> String {
    for cause in error.chain() {
        if let Some(dispatch) = cause.downcast_ref::<wayland_client::DispatchError>() {
            return match dispatch {
                wayland_client::DispatchError::BadMessage {
                    sender_id,
                    interface,
                    opcode,
                } => format!("badmsg {interface}@{sender_id} op={opcode}"),
                wayland_client::DispatchError::Backend(backend) => {
                    overlay_wayland_error_kind(backend)
                }
            };
        }
        if let Some(backend) = cause.downcast_ref::<wayland_client::backend::WaylandError>() {
            return overlay_wayland_error_kind(backend);
        }
        if cause
            .downcast_ref::<wayland_client::backend::InvalidId>()
            .is_some()
        {
            return "invalid-id".to_owned();
        }
    }
    format!("other {}", compact_error_message(&error.to_string(), 35))
}

#[must_use]
fn overlay_wayland_error_kind(error: &wayland_client::backend::WaylandError) -> String {
    match error {
        wayland_client::backend::WaylandError::Io(err) => {
            format!("io {:?} os={:?}", err.kind(), err.raw_os_error())
        }
        wayland_client::backend::WaylandError::Protocol(protocol) => {
            let message = compact_error_message(&protocol.message, 24);
            format!(
                "proto {}@{} code={} msg={}",
                protocol.object_interface, protocol.object_id, protocol.code, message
            )
        }
    }
}

pub type SlotId = u64;

#[expect(missing_debug_implementations)]
pub struct SlotTable {
    next: SlotId,
    map: HashMap<SlotId, WidgetSlot>,
}

impl SlotTable {
    #[must_use]
    pub fn new() -> Self {
        Self {
            next: 0,
            map: HashMap::new(),
        }
    }
    pub fn insert(&mut self, slot: WidgetSlot) -> SlotId {
        let id = self.next;
        self.next += 1;
        self.map.insert(id, slot);
        id
    }
    pub fn remove(&mut self, id: &SlotId) -> Option<WidgetSlot> {
        self.map.remove(id)
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = &WidgetSlot> {
        self.map.values()
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&SlotId, &mut WidgetSlot)> {
        self.map.iter_mut()
    }
}

pub fn drain_and_shutdown(
    slots: &mut SlotTable,
    shared: &mut SharedHost,
    renderer: &mut FemtoVgRenderer,
) -> usize {
    let ids: Vec<SlotId> = slots.iter_mut().map(|(id, _)| *id).collect();
    let mut count = 0;
    for id in ids {
        if let Some(slot) = slots.remove(&id) {
            tracing::info!(
                peer_pid = ?slot.peer_pid,
                wasm = %slot.wasm_basename,
                "slot drained on fatal exit",
            );
            slot.shutdown(shared, renderer);
            count += 1;
        }
    }
    count
}

pub fn drain_if_err<T>(
    result: Result<T, FatalError>,
    slots: &mut SlotTable,
    shared: &mut SharedHost,
    renderer: &mut FemtoVgRenderer,
) -> Result<T, FatalError> {
    if result.is_err() {
        let drained = drain_and_shutdown(slots, shared, renderer);
        tracing::warn!(drained, "fatal exit drained slots");
    }
    result
}

const LISTENER_INDEX: usize = 0;

/// Publish this host's live cache tokens to its GC-root file. Best-effort: a
/// write failure is logged, never fatal — the next tick retries.
fn publish_gc_root(slots: &SlotTable) {
    let tokens: Vec<String> = slots.iter().filter_map(|s| s.cache_token.clone()).collect();
    if let Err(err) = cache_gc::write_root(&tokens) {
        tracing::warn!(%err, "failed to publish widget cache GC root");
    }
}

/// Accept every connection currently queued on the (non-blocking) listener.
///
/// One `POLLIN` can back several pending connections — a scene or post-upgrade
/// restart reconnects every thin at once — so accepting one per wake would
/// stall the rest behind the poll timeout. Draining to `WouldBlock` also lets
/// the pre-exit sweep pick up a connection queued during the last slot's
/// teardown, which the peer already saw `connect()` succeed for.
pub fn accept_pending(listener: &UnixListener) -> Result<Vec<UnixStream>, FatalError> {
    let mut pending = Vec::new();
    loop {
        match listener.accept() {
            Ok((client, _)) => pending.push(client),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) if e.raw_os_error() == Some(libc::EINTR) => continue,
            Err(e) => return Err(FatalError::AcceptFailed(e)),
        }
    }
    Ok(pending)
}

/// Drain the accept backlog and load each connection into a slot, folding the
/// burst's outcome into the lifetime. Shared by the poll-driven accept and the
/// final pre-exit sweep so both paths tally slots the same way.
fn drain_accept_burst(
    listener: &ListenSocket,
    slots: &mut SlotTable,
    shared: &mut SharedHost,
    lifetime: &mut HostLifetime,
    next_gc: &mut Instant,
) -> Result<(), FatalError> {
    let mut loaded = 0_usize;
    let mut rejected = 0_usize;
    for client in accept_pending(listener.as_listener())? {
        match accept_and_load(client, shared) {
            Ok(slot) => {
                let peer_pid = slot.peer_pid;
                let wasm = slot.wasm_basename.clone();
                let slot_id = slots.insert(slot);
                tracing::info!(slot_id, ?peer_pid, wasm = %wasm, "slot inserted");
                loaded += 1;
            }
            Err(e) => {
                rejected += 1;
                tracing::warn!(?e, "load failed; slot rejected");
            }
        }
    }
    if loaded > 0 {
        // Pull the next GC in so the new tokens publish soon
        // (a startup/scene burst coalesces into one).
        *next_gc = (*next_gc).min(Instant::now() + GC_SETTLE_DELAY);
    }
    lifetime.note_accept_burst(loaded, rejected, slots.len());
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "the main loop body is a single coherent dispatch cycle; splitting would obscure the control flow"
)]
fn run_loop(
    shared: &mut SharedHost,
    renderer: &mut FemtoVgRenderer,
    listener: &ListenSocket,
    slots: &mut SlotTable,
    overlays: &mut Vec<HostedOverlay>,
) -> Result<(), FatalError> {
    listener
        .set_nonblocking()
        .map_err(FatalError::ControlSocketBindFailed)?;

    let renderer_raw: *mut dyn Renderer = &raw mut *renderer;
    let renderer_ptr = NonNull::new(renderer_raw)
        .expect("BUG: &raw mut from a live &mut produces a non-null pointer");

    // Pay overlays' one-time renderer setup (SVG icons, glyph atlas) before the
    // event loop so the first screen-edge reveal is not slowed by it.
    for overlay in overlays.iter_mut() {
        if let Err(e) = crate::overlays::prewarm_hosted_overlay(overlay, renderer_ptr, shared) {
            tracing::warn!("overlay prewarm failed: {e}");
        }
    }

    let mut lifetime = HostLifetime::new();

    // Defer the first publish + reconcile until widgets load.
    // An empty root now would let a peer's sweep wipe buckets we re-use;
    // the previous run's root protects them until the scheduled pass.
    let mut next_gc = Instant::now() + GC_SETTLE_DELAY;

    while lifetime.should_continue(slots.len(), overlays.iter().any(HostedOverlay::running)) {
        let poll_now = Instant::now();
        let slot_ms = compute_poll_timeout(slots, poll_now);
        let mut wake = u64::try_from(slot_ms).ok().map(Duration::from_millis);
        for overlay in overlays.iter() {
            if let Some(d) = overlay.poll_timeout(poll_now) {
                wake = Some(wake.map_or(d, |w| w.min(d)));
            }
        }
        // -1 = block-forever sentinel; only when neither slots nor overlays want a wake.
        let slot_timeout = wake.map_or(-1, |d| i32::try_from(d.as_millis()).unwrap_or(i32::MAX));
        // Cap the wait by the next GC tick so an idle host still wakes to tick.
        let gc_ms = i32::try_from(
            next_gc
                .saturating_duration_since(Instant::now())
                .as_millis(),
        )
        .unwrap_or(i32::MAX);
        let timeout_ms = if slot_timeout < 0 {
            gc_ms
        } else {
            slot_timeout.min(gc_ms)
        };

        let mut pollfds: Vec<libc::pollfd> =
            Vec::with_capacity(1 + 2 * slots.len() + overlays.len());
        pollfds.push(libc::pollfd {
            fd: listener.as_listener().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        });
        for slot in slots.iter() {
            pollfds.push(libc::pollfd {
                fd: slot.surface.fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
            pollfds.push(libc::pollfd {
                fd: slot.control_socket.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }
        for overlay in overlays.iter() {
            pollfds.push(libc::pollfd {
                fd: overlay.connection_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }

        match unsafe {
            libc::poll(
                pollfds.as_mut_ptr(),
                pollfds.len() as libc::nfds_t,
                timeout_ms,
            )
        } {
            n if n >= 0 => {}
            _ => match classify_poll_errno(&std::io::Error::last_os_error()) {
                PollDecision::Retry => continue,
                PollDecision::Fatal(e) => return Err(FatalError::PollFailed(e)),
            },
        }

        classify_listener_revents(pollfds[LISTENER_INDEX].revents)?;

        if (pollfds[LISTENER_INDEX].revents & libc::POLLIN) != 0 {
            drain_accept_burst(listener, slots, shared, &mut lifetime, &mut next_gc)?;
        }

        let mut to_teardown: Vec<SlotId> = Vec::new();
        for (id, slot) in slots.iter_mut() {
            if slot.dispatch_wayland_events().is_err() {
                to_teardown.push(*id);
                continue;
            }
            slot.refresh_network();
            slot.reclaim_retired_render_targets(shared);
            if slot.dispatch_control_socket().is_err() {
                to_teardown.push(*id);
                continue;
            }
            let now = Instant::now();
            slot.apply_lifecycle(now, &shared.egl);
            slot.advance_runtime_time(chrono::Local::now().fixed_offset(), now);
            if let Err(e) = slot.runtime.poll_deliveries_with_renderer(renderer_ptr) {
                tracing::error!(
                    peer_pid = ?slot.peer_pid, wasm = %slot.wasm_basename, error = ?e,
                    "widget delivery trapped; tearing down slot"
                );
                to_teardown.push(*id);
                continue;
            }
            slot.refresh_next_runtime_frame_after_delivery(now);
            if slot.flush_led_requests().is_err() {
                to_teardown.push(*id);
                continue;
            }
        }

        for overlay in overlays.iter_mut() {
            if let Err(e) = overlay.dispatch(&shared.egl) {
                // A persistent dispatch error that never delivers Closed would
                // otherwise log every pass. Treat it as terminal; the cleanup
                // loop below drops + cleans it up.
                tracing::error!("ovl-dsp {}", overlay_dispatch_error_kind(&e));
                overlay.mark_failed();
            }
        }

        let now = Instant::now();
        for (id, slot) in slots.iter_mut() {
            if to_teardown.contains(id) {
                continue;
            }
            if !slot.needs_render(now) {
                continue;
            }
            let delta_ms = slot.tick_delta(now);
            slot.advance_runtime_time(chrono::Local::now().fixed_offset(), now);

            // Bind the guard — an unbound `.entered()` exits the span at once,
            // before the render whose panics it should tag with the widget.
            let _span = tracing::info_span!("widget", wasm = %slot.wasm_basename).entered();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                slot.render(renderer_ptr, delta_ms, shared)
            }));
            match result {
                Ok(Ok(bmc_wasm_runtime::RenderStatus::Ok)) => {}
                Ok(Ok(bmc_wasm_runtime::RenderStatus::FuelExhausted)) => tracing::warn!(
                    peer_pid = ?slot.peer_pid, wasm = %slot.wasm_basename,
                    "widget exceeded fuel budget"
                ),
                Ok(Ok(bmc_wasm_runtime::RenderStatus::Dead)) => {
                    if shared.is_context_lost() {
                        return Err(FatalError::EglContextLost);
                    }
                    tracing::error!(
                        peer_pid = ?slot.peer_pid, wasm = %slot.wasm_basename,
                        "widget runtime dead; tearing down slot"
                    );
                    to_teardown.push(*id);
                }
                Ok(Err(e)) => {
                    if shared.is_context_lost() {
                        return Err(FatalError::EglContextLost);
                    }
                    tracing::error!(
                        peer_pid = ?slot.peer_pid, wasm = %slot.wasm_basename, error = ?e,
                        "widget render failed; tearing down slot"
                    );
                    to_teardown.push(*id);
                }
                Err(_) => {
                    if shared.is_context_lost() {
                        return Err(FatalError::EglContextLost);
                    }
                    tracing::error!(
                        peer_pid = ?slot.peer_pid, wasm = %slot.wasm_basename,
                        "widget render panicked; tearing down slot"
                    );
                    to_teardown.push(*id);
                }
            }

            // A widget can emit LED requests from render-time input handling,
            // which land after the pre-render flush. Drain them now so an
            // otherwise-idle widget doesn't strand them on poll(-1) until some
            // unrelated event wakes the loop.
            if !to_teardown.contains(id) && slot.flush_led_requests().is_err() {
                to_teardown.push(*id);
            }
        }

        for overlay in overlays.iter_mut() {
            overlay.tick(now);
            if overlay.needs_hide() {
                if let Err(e) = overlay.hide(&shared.egl) {
                    tracing::error!("overlay hide error, dropping overlay: {e}");
                    overlay.mark_failed();
                }
                continue;
            }
            if overlay.needs_render(now)
                && let Err(e) =
                    crate::overlays::render_hosted_overlay(overlay, renderer_ptr, shared, now)
            {
                // Mirror the slot render-error path: a lost EGL context is
                // fatal and must propagate, not be swallowed until the next
                // widget render notices it.
                if shared.is_context_lost() {
                    return Err(FatalError::EglContextLost);
                }
                tracing::error!("overlay render error, dropping overlay: {e}");
                overlay.mark_failed();
            }
            if overlay.needs_cache_refresh()
                && let Err(e) =
                    crate::overlays::refresh_overlay_cache(overlay, renderer_ptr, shared)
            {
                if shared.is_context_lost() {
                    return Err(FatalError::EglContextLost);
                }
                tracing::error!("overlay cache refresh error, dropping overlay: {e}");
                overlay.mark_failed();
            }
            overlay.forward_settings_requests();
            overlay.forward_alarm_requests();
        }
        // Drop overlays whose client closed or that hit a terminal error,
        // shutting down each first so its GPU resources are freed. A plain
        // `retain` would Drop them without `shutdown(egl)` and leak.
        let mut idx = 0;
        while idx < overlays.len() {
            if !overlays[idx].running() || overlays[idx].is_failed() {
                overlays[idx].shutdown(&shared.egl);
                overlays.remove(idx);
            } else {
                idx += 1;
            }
        }

        if !to_teardown.is_empty() {
            for id in to_teardown {
                if let Some(slot) = slots.remove(&id) {
                    tracing::info!(peer_pid = ?slot.peer_pid, wasm = %slot.wasm_basename, "slot teardown");
                    slot.shutdown(shared, renderer);
                }
            }
        }

        // The loop-top exit check runs before the next poll()/accept(), so a
        // thin that connected into the backlog while this iteration tore down
        // the last slot would be orphaned: its connect() succeeded, but it is
        // never accepted and the dropped listener resets it. Sweep the backlog
        // once more before honoring a slots-driven exit — a queued connection
        // revives the host instead of restarting it from scratch.
        if !lifetime.should_continue(slots.len(), overlays.iter().any(HostedOverlay::running)) {
            drain_accept_burst(listener, slots, shared, &mut lifetime, &mut next_gc)?;
        }

        // Heartbeat + sweep on the next_gc deadline; republishing also picks up
        // teardowns. After a run the deadline resets a full period out.
        if Instant::now() >= next_gc {
            publish_gc_root(slots);
            let stats = cache_gc::reconcile();
            tracing::debug!(?stats, "widget asset cache GC");
            next_gc = Instant::now() + cache_gc::gc_period();
        }
    }
    Ok(())
}

pub fn run_with_slots(
    shared: &mut SharedHost,
    renderer: &mut FemtoVgRenderer,
    listener: &ListenSocket,
    slots: &mut SlotTable,
) -> Result<(), FatalError> {
    let mut overlays = crate::overlays::build_overlays(&shared.egl);
    let result = run_loop(shared, renderer, listener, slots, &mut overlays);
    for overlay in &mut overlays {
        overlay.shutdown(&shared.egl);
    }
    drain_if_err(result, slots, shared, renderer)
}

pub fn run(
    shared: &mut SharedHost,
    renderer: &mut FemtoVgRenderer,
    listener: &ListenSocket,
) -> Result<(), FatalError> {
    let mut slots = SlotTable::new();
    run_with_slots(shared, renderer, listener, &mut slots)
}

#[cfg(test)]
mod tests {
    use super::{compact_error_message, overlay_dispatch_error_kind};

    #[test]
    fn compact_error_message_caps_long_protocol_text() {
        assert_eq!(compact_error_message("abcdef", 3), "abc");
    }

    #[test]
    fn overlay_dispatch_error_kind_keeps_protocol_details_short() {
        let protocol = wayland_client::backend::protocol::ProtocolError {
            code: 7,
            object_id: 42,
            object_interface: "wl_buffer".to_owned(),
            message: "message that is deliberately too long for device logs".to_owned(),
        };
        let error = anyhow::Error::new(wayland_client::backend::WaylandError::Protocol(protocol))
            .context("poll_dispatch");

        assert_eq!(
            overlay_dispatch_error_kind(&error),
            "proto wl_buffer@42 code=7 msg=message that is delibera"
        );
    }

    #[test]
    fn overlay_dispatch_error_kind_reports_io_class() {
        let error = anyhow::Error::new(wayland_client::backend::WaylandError::Io(
            std::io::Error::from_raw_os_error(libc::EPIPE),
        ));

        assert_eq!(
            overlay_dispatch_error_kind(&error),
            "io BrokenPipe os=Some(32)"
        );
    }

    #[test]
    fn overlay_dispatch_error_kind_keeps_unknown_context_short() {
        let error = anyhow::anyhow!(
            "resize failed because the reusable overlay target could not allocate the requested buffer"
        );

        assert_eq!(
            overlay_dispatch_error_kind(&error),
            "other resize failed because the reusable "
        );
    }
}
