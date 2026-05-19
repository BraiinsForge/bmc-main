// Copyright (C) 2026  Braiins Systems s.r.o.

use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::render_target::{RenderTarget, RenderTargetFactory};

const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(1);

pub trait LifecycleEgl {
    fn as_egl_context(&self) -> &bmc_widget::egl::EglContext;
}

pub trait LifecycleSurface {
    fn as_deck_widget_surface(&self) -> &bmc_widget::surface::DeckWidgetSurfaceClient;
    fn mint_wl_buffer(
        &self,
        dmabuf: &bmc_widget::egl::DmaBufInfo,
    ) -> Result<wayland_client::protocol::wl_buffer::WlBuffer, String>;
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
    matches!(s, LifecycleState::Visible | LifecycleState::Leaving)
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
    pub surface: &'a dyn LifecycleSurface,
    pub render_target: &'a mut Option<RenderTarget>,
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
        &self,
        dmabuf: &bmc_widget::egl::DmaBufInfo,
    ) -> Result<wayland_client::protocol::wl_buffer::WlBuffer, String> {
        self.mint_wl_buffer_via_dmabuf(dmabuf)
            .map_err(|e| format!("{e:?}"))
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

    pub fn on_event(&mut self, new_target: LifecycleState) -> LifecycleEventEffect {
        let previous_target = self.target;
        self.target = new_target;
        LifecycleEventEffect {
            request_render: has_render_target(new_target) && previous_target != new_target,
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
                    tracing::info!(
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
            ctx.factory.destroy(target, ctx.egl);
            self.current = self.target;
            self.blocked = false;
            self.retry_at = None;
            tracing::info!(state = ?self.current, "render target released");
        } else {
            self.current = self.target;
            // If we were blocked while owing an allocation but the compositor has since
            // demoted the slot back to a buffer-free target, `target_needs && !current_has`
            // is false above; the supersession itself resolves the block, so clear it.
            self.blocked = false;
            self.retry_at = None;
        }
    }
}
