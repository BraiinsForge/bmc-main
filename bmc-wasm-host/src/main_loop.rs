// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use bmc_render::renderer::Renderer;

use crate::control::{ListenSocket, accept_and_load};
use crate::host::SharedHost;
use crate::slot::WidgetSlot;

const GRACE_DURATION: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub struct HostLifetime {
    ever_had_slot: bool,
    last_disconnect: Option<Instant>,
}

impl HostLifetime {
    #[must_use]
    pub fn new() -> Self {
        Self {
            ever_had_slot: false,
            last_disconnect: None,
        }
    }

    pub fn note_accept(&mut self) {
        self.ever_had_slot = true;
        self.last_disconnect = None;
    }

    pub fn note_disconnect(&mut self, now: Instant) {
        self.ever_had_slot = true;
        self.last_disconnect = Some(now);
    }

    pub fn note_failed_load(&mut self, now: Instant) {
        self.ever_had_slot = true;
        self.last_disconnect = Some(now);
    }

    #[must_use]
    pub fn should_continue(&self, slots_len: usize, now: Instant) -> bool {
        if slots_len > 0 {
            return true;
        }
        if !self.ever_had_slot {
            return true;
        }
        match self.last_disconnect {
            None => true,
            Some(t) => now.duration_since(t) < GRACE_DURATION,
        }
    }

    #[must_use]
    pub fn poll_timeout_contribution(&self, now: Instant) -> Option<Duration> {
        let t = self.last_disconnect?;
        Some(GRACE_DURATION.saturating_sub(now.duration_since(t)))
    }
}

/// Pure-function inputs that `compute_poll_timeout_from_inputs` consumes per slot.
///
/// Extracted so the timeout policy can be unit-tested in isolation without standing up
/// a real `WidgetSlot` (which needs an EGL context, a Wayland connection, and a wasmi
/// store). The thin `compute_poll_timeout` wrapper below collects these from each slot
/// in production.
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

/// Pure: fold per-slot inputs and the lifetime grace remainder into a single
/// `poll(2)` timeout in milliseconds. `-1` is `poll(2)`'s indefinite-block sentinel,
/// returned when nothing contributes a finite value.
#[must_use]
pub fn compute_poll_timeout_from_inputs(
    slots: &[SlotPollInputs],
    grace_remaining: Option<Duration>,
) -> i32 {
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

        // Animation-driven wakeups only contribute when the lifecycle state actually
        // honors frame callbacks (Visible | Leaving). Entering renders once on a dirty
        // surface and then sleeps; advertising its `next_frame_delay` here would wake
        // the host for ticks that `needs_render` is going to discard.
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
    if slots.is_empty()
        && let Some(d) = grace_remaining
    {
        push(&mut best, d);
    }
    match best {
        None => -1,
        Some(d) => i32::try_from(d.as_millis()).unwrap_or(i32::MAX),
    }
}

/// Production wrapper: gather every slot's inputs and forward to the pure core.
#[must_use]
pub fn compute_poll_timeout(slots: &SlotTable, lifetime: &HostLifetime, now: Instant) -> i32 {
    let inputs: Vec<SlotPollInputs> = slots.iter().map(|s| s.poll_inputs(now)).collect();
    compute_poll_timeout_from_inputs(&inputs, lifetime.poll_timeout_contribution(now))
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

const LISTENER_INDEX: usize = 0;

#[expect(
    clippy::too_many_lines,
    reason = "the main loop body is a single coherent dispatch cycle; splitting would obscure the control flow"
)]
pub fn run(shared: &mut SharedHost, listener: &ListenSocket) -> Result<(), FatalError> {
    listener
        .set_nonblocking()
        .map_err(FatalError::ControlSocketBindFailed)?;

    let renderer_raw: *mut dyn Renderer = core::ptr::addr_of_mut!(shared.renderer);
    let renderer_ptr = NonNull::new(renderer_raw)
        .expect("BUG: addr_of_mut! is a compiler intrinsic that cannot return a null pointer");

    let mut slots = SlotTable::new();
    let mut lifetime = HostLifetime::new();

    while lifetime.should_continue(slots.len(), Instant::now()) {
        let timeout_ms = compute_poll_timeout(&slots, &lifetime, Instant::now());

        let mut pollfds: Vec<libc::pollfd> = Vec::with_capacity(1 + 2 * slots.len());
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
            match listener.as_listener().accept() {
                Ok((client, _)) => match accept_and_load(client, shared) {
                    Ok(slot) => {
                        let peer_pid = slot.peer_pid;
                        let wasm = slot.wasm_basename.clone();
                        let slot_id = slots.insert(slot);
                        tracing::info!(slot_id, peer_pid, wasm = %wasm, "slot inserted");
                        lifetime.note_accept();
                    }
                    Err(e) => {
                        if slots.is_empty() {
                            lifetime.note_failed_load(Instant::now());
                        }
                        tracing::warn!(?e, "load failed; slot rejected");
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) if e.raw_os_error() == Some(libc::EINTR) => {}
                Err(e) => return Err(FatalError::AcceptFailed(e)),
            }
        }

        let mut to_teardown: Vec<SlotId> = Vec::new();
        for (id, slot) in slots.iter_mut() {
            if slot.dispatch_wayland_events().is_err() {
                to_teardown.push(*id);
                continue;
            }
            if slot.dispatch_control_socket().is_err() {
                to_teardown.push(*id);
                continue;
            }
            slot.apply_lifecycle(Instant::now(), shared);
            slot.runtime.poll_deliveries_with_renderer(renderer_ptr);
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
            slot.runtime
                .set_time(chrono::Local::now().fixed_offset(), slot.monotonic_ms(now));

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                slot.render(renderer_ptr, delta_ms, shared)
            }));
            match result {
                Ok(Ok(bmc_wasm_runtime::RenderStatus::Ok)) => {}
                Ok(Ok(bmc_wasm_runtime::RenderStatus::FuelExhausted)) => tracing::warn!(
                    peer_pid = slot.peer_pid, wasm = %slot.wasm_basename,
                    "widget exceeded fuel budget"
                ),
                Ok(Ok(bmc_wasm_runtime::RenderStatus::Dead) | Err(_)) | Err(_) => {
                    if shared.egl.is_context_lost() {
                        return Err(FatalError::EglContextLost);
                    }
                    to_teardown.push(*id);
                }
            }
        }

        if !to_teardown.is_empty() {
            for id in to_teardown {
                if let Some(slot) = slots.remove(&id) {
                    tracing::info!(peer_pid = slot.peer_pid, wasm = %slot.wasm_basename, "slot teardown");
                    slot.shutdown(shared);
                }
            }
            if slots.is_empty() {
                lifetime.note_disconnect(Instant::now());
            }
        }
    }
    Ok(())
}
