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

use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::render_target::{RenderTarget, RenderTargetCleanup, RenderTargetFactory};

const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(1);

pub trait LifecycleEgl {
    fn as_egl_context(&self) -> &bmc_widget::egl::EglContext;
}

pub trait LifecycleSurface {
    fn as_deck_widget_surface(&self) -> &bmc_widget::surface::DeckWidgetSurfaceClient;
    fn mint_wl_buffer(
        &mut self,
        dmabuf: &bmc_widget::egl::DmaBufInfo,
        slot: usize,
    ) -> Result<wayland_client::protocol::wl_buffer::WlBuffer, String>;
    fn destroy_minted_wl_buffer(&mut self, buffer: wayland_client::protocol::wl_buffer::WlBuffer);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleState {
    Dormant,
    Prepared,
    Entering,
    Visible,
    Leaving,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifecycleEventEffect {
    pub request_render: bool,
}

#[must_use]
pub fn has_render_target(s: LifecycleState) -> bool {
    matches!(
        s,
        LifecycleState::Prepared
            | LifecycleState::Entering
            | LifecycleState::Visible
            | LifecycleState::Leaving
    )
}

#[must_use]
pub fn should_render(s: LifecycleState) -> bool {
    has_render_target(s)
}

#[must_use]
pub fn frame_callback_enabled(s: LifecycleState) -> bool {
    matches!(s, LifecycleState::Visible)
}

/// Guest lifecycle hook a committed state transition fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleHook {
    /// Left `Dormant` — restore before the first frame.
    Wake,
    /// Entered `Dormant` — release off-scene resources.
    Sleep,
}

/// Hook for a committed `previous → current` transition — the `has_render_target` flip.
#[must_use]
pub fn lifecycle_hook(previous: LifecycleState, current: LifecycleState) -> Option<LifecycleHook> {
    match (has_render_target(previous), has_render_target(current)) {
        (false, true) => Some(LifecycleHook::Wake),
        (true, false) => Some(LifecycleHook::Sleep),
        _ => None,
    }
}

#[derive(Debug)]
pub struct LifecycleStateMachine {
    current: LifecycleState,
    target: LifecycleState,
    blocked: bool,
    retry_at: Option<Instant>,
}

#[expect(missing_debug_implementations)]
pub struct SlotApplyCtx<'a> {
    pub factory: &'a Rc<dyn RenderTargetFactory>,
    pub egl: &'a dyn LifecycleEgl,
    pub surface: &'a mut dyn LifecycleSurface,
    pub render_target: &'a mut Option<RenderTarget>,
    pub retired_render_targets: &'a mut Vec<RenderTarget>,
    pub width: u32,
    pub height: u32,
}

impl LifecycleEgl for bmc_widget::egl::EglContext {
    fn as_egl_context(&self) -> &Self {
        self
    }
}

impl LifecycleSurface for bmc_widget::surface::DeckWidgetSurfaceClient {
    fn as_deck_widget_surface(&self) -> &Self {
        self
    }

    fn mint_wl_buffer(
        &mut self,
        dmabuf: &bmc_widget::egl::DmaBufInfo,
        slot: usize,
    ) -> Result<wayland_client::protocol::wl_buffer::WlBuffer, String> {
        bmc_widget::surface::DeckWidgetSurfaceClient::mint_wl_buffer_for_slot(self, dmabuf, slot)
            .map_err(|e| format!("{e:?}"))
    }

    fn destroy_minted_wl_buffer(&mut self, buffer: wayland_client::protocol::wl_buffer::WlBuffer) {
        bmc_widget::surface::DeckWidgetSurfaceClient::destroy_minted_wl_buffer(self, buffer);
    }
}

impl LifecycleStateMachine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            current: LifecycleState::Dormant,
            target: LifecycleState::Dormant,
            blocked: false,
            retry_at: None,
        }
    }

    #[must_use]
    pub fn current(&self) -> LifecycleState {
        self.current
    }
    #[must_use]
    pub fn target(&self) -> LifecycleState {
        self.target
    }
    #[must_use]
    pub fn blocked(&self) -> bool {
        self.blocked
    }
    #[must_use]
    pub fn retry_at(&self) -> Option<Instant> {
        self.retry_at
    }

    #[must_use]
    pub fn render_target_change_ready(&self, now: Instant) -> bool {
        if has_render_target(self.current) == has_render_target(self.target) {
            return false;
        }
        !self.blocked
            || !has_render_target(self.target)
            || self.retry_at.is_some_and(|retry_at| now >= retry_at)
    }

    pub fn on_event(&mut self, new_target: LifecycleState) -> LifecycleEventEffect {
        let previous_target = self.target;
        self.target = new_target;
        LifecycleEventEffect {
            request_render: has_render_target(new_target)
                && !has_render_target(previous_target)
                && previous_target != new_target,
        }
    }

    #[must_use]
    pub fn needs_retry(&self, now: Instant) -> bool {
        self.blocked && self.retry_at.is_some_and(|t| now >= t)
    }

    pub fn apply(&mut self, ctx: &mut SlotApplyCtx<'_>, now: Instant) {
        let target_needs = has_render_target(self.target);
        let current_has = has_render_target(self.current);

        // Spec § Allocation failure behavior: a blocked slot retries on a 1 s timer, not
        // every main-loop iteration. Without this gate `apply_lifecycle` (called every
        // iteration from `main_loop::run`) would hammer the factory thousands of times
        // per second while CMA is exhausted. The skip only applies while we still owe an
        // allocation (target wants a buffer, we don't have one); if the compositor has
        // since demoted the slot back to the buffer-free state, target_needs is false and
        // execution falls through to the release / no-op branches below, which clear
        // `blocked` as a side-effect of the transition.
        if self.blocked && target_needs && !current_has && self.retry_at.is_some_and(|t| now < t) {
            return;
        }

        if target_needs && !current_has {
            assert!(
                ctx.render_target.is_none(),
                "BUG: allocating render target while one already exists"
            );
            match ctx
                .factory
                .allocate(ctx.egl, ctx.surface, ctx.width, ctx.height)
            {
                Ok(t) => {
                    *ctx.render_target = Some(t);
                    self.current = self.target;
                    self.blocked = false;
                    self.retry_at = None;
                    tracing::debug!(
                        state = ?self.current,
                        w = ctx.width,
                        h = ctx.height,
                        "render target allocated"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        ?e,
                        w = ctx.width,
                        h = ctx.height,
                        "render target alloc failed"
                    );
                    self.blocked = true;
                    self.retry_at = Some(now + RETRY_AFTER_FAILURE);
                }
            }
        } else if !target_needs && current_has {
            assert!(
                ctx.render_target.is_some(),
                "BUG: releasing render target while none exists"
            );
            let target = ctx
                .render_target
                .take()
                .expect("BUG: releasing render target while none exists");
            let mut target = target;
            match ctx
                .factory
                .destroy_released_slots(&mut target, ctx.egl, ctx.surface)
            {
                RenderTargetCleanup::Complete => {}
                RenderTargetCleanup::PendingRelease => {
                    ctx.retired_render_targets.push(target);
                }
            }
            self.current = self.target;
            self.blocked = false;
            self.retry_at = None;
            tracing::debug!(state = ?self.current, "render target released");
        } else {
            self.current = self.target;
            // If we were blocked while owing an allocation but the compositor has since
            // demoted the slot back to a buffer-free target, `target_needs && !current_has`
            // is false above; the supersession itself resolves the block, so clear it.
            self.blocked = false;
            self.retry_at = None;
        }

        // Sole compaction trigger. `apply` runs every main-loop iteration after
        // `dispatch_wayland_events` has marked released slots, so this catches
        // both moments a `Prepared` slot can shed a buffer: right after the
        // allocation above, and the iteration after a release frees the spare.
        // The placement after release-marking is load-bearing — if `apply` ever
        // gains a no-transition early return, release-driven compaction must be
        // re-armed explicitly.
        if self.current == LifecycleState::Prepared
            && let Some(target) = ctx.render_target.as_mut()
        {
            ctx.factory
                .compact_for_prepared(target, ctx.egl, ctx.surface);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LifecycleState::{Dormant, Entering, Leaving, Prepared, Visible};
    use super::{LifecycleHook, LifecycleState, lifecycle_hook};

    const RENDER_STATES: [LifecycleState; 4] = [Prepared, Entering, Visible, Leaving];

    #[test]
    fn wake_fires_on_leaving_dormant() {
        for to in RENDER_STATES {
            assert_eq!(
                lifecycle_hook(Dormant, to),
                Some(LifecycleHook::Wake),
                "Dormant -> {to:?}"
            );
        }
    }

    #[test]
    fn dormant_fires_on_entering_dormant() {
        for from in RENDER_STATES {
            assert_eq!(
                lifecycle_hook(from, Dormant),
                Some(LifecycleHook::Sleep),
                "{from:?} -> Dormant"
            );
        }
    }

    #[test]
    fn no_hook_between_render_target_states() {
        for from in RENDER_STATES {
            for to in RENDER_STATES {
                assert_eq!(lifecycle_hook(from, to), None, "{from:?} -> {to:?}");
            }
        }
    }

    #[test]
    fn no_hook_when_staying_dormant() {
        assert_eq!(lifecycle_hook(Dormant, Dormant), None);
    }
}
