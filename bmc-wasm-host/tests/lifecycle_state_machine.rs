// Copyright (C) 2026  Braiins Systems s.r.o.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bmc_wasm_host::lifecycle::{
    LifecycleEgl, LifecycleState, LifecycleStateMachine, LifecycleSurface, SlotApplyCtx,
};
use bmc_wasm_host::render_target::{RenderTarget, RenderTargetError, RenderTargetFactory};

struct StubEgl;
impl LifecycleEgl for StubEgl {
    fn as_egl_context(&self) -> &bmc_widget::egl::EglContext {
        unimplemented!(
            "MockFactory never calls as_egl_context — the production path is exercised in Stage 6"
        )
    }
}

struct StubSurface;
impl LifecycleSurface for StubSurface {
    fn as_deck_widget_surface(&self) -> &bmc_widget::surface::DeckWidgetSurfaceClient {
        unimplemented!(
            "MockFactory never calls as_deck_widget_surface — the production path is exercised in Stage 6"
        )
    }
    fn mint_wl_buffer(
        &self,
        _: &bmc_widget::egl::DmaBufInfo,
    ) -> Result<wayland_client::protocol::wl_buffer::WlBuffer, String> {
        unimplemented!(
            "MockFactory never calls mint_wl_buffer — the production path is exercised in Stage 6"
        )
    }
}

struct MockFactory {
    fail_next: Cell<u32>,
    alloc_calls: Cell<u32>,
    destroy_calls: Cell<u32>,
}

impl MockFactory {
    fn new() -> Self {
        Self {
            fail_next: Cell::new(0),
            alloc_calls: Cell::new(0),
            destroy_calls: Cell::new(0),
        }
    }
    fn fail(&self, n: u32) {
        self.fail_next.set(n);
    }
}

impl RenderTargetFactory for MockFactory {
    fn allocate(
        &self,
        _: &dyn LifecycleEgl,
        _: &dyn LifecycleSurface,
        _: u32,
        _: u32,
    ) -> Result<RenderTarget, RenderTargetError> {
        self.alloc_calls.set(self.alloc_calls.get() + 1);
        let pending = self.fail_next.get();
        if pending > 0 {
            self.fail_next.set(pending - 1);
            return Err(RenderTargetError::Wayland("mock fail".into()));
        }
        Ok(RenderTarget::new_stub(128, 128))
    }

    fn destroy(&self, _: RenderTarget, _: &dyn LifecycleEgl) {
        self.destroy_calls.set(self.destroy_calls.get() + 1);
    }
}

fn ctx_no_target<'a>(
    factory: &'a Rc<dyn RenderTargetFactory>,
    target: &'a mut Option<RenderTarget>,
    egl: &'a StubEgl,
    surface: &'a StubSurface,
) -> SlotApplyCtx<'a> {
    SlotApplyCtx {
        factory,
        egl,
        surface,
        render_target: target,
        width: 128,
        height: 128,
    }
}

#[test]
#[should_panic(expected = "BUG: allocating render target while one already exists")]
fn dormant_to_target_owning_state_panics_if_target_already_exists() {
    let mock: Rc<MockFactory> = Rc::new(MockFactory::new());
    let factory: Rc<dyn RenderTargetFactory> = mock.clone();
    mock.fail(1);

    let mut target = Some(RenderTarget::new_stub(128, 128));
    let egl = StubEgl;
    let surface = StubSurface;

    let mut sm = LifecycleStateMachine::new();
    sm.on_event(LifecycleState::Entering);
    sm.apply(
        &mut ctx_no_target(&factory, &mut target, &egl, &surface),
        Instant::now(),
    );
}

#[test]
#[should_panic(expected = "BUG: releasing render target while none exists")]
fn target_owning_to_dormant_panics_if_target_is_missing() {
    let mock: Rc<MockFactory> = Rc::new(MockFactory::new());
    let factory: Rc<dyn RenderTargetFactory> = mock.clone();

    let mut target: Option<RenderTarget> = None;
    let egl = StubEgl;
    let surface = StubSurface;

    let mut sm = LifecycleStateMachine::new();
    sm.on_event(LifecycleState::Entering);
    sm.apply(
        &mut ctx_no_target(&factory, &mut target, &egl, &surface),
        Instant::now(),
    );
    target = None;

    sm.on_event(LifecycleState::Dormant);
    sm.apply(
        &mut ctx_no_target(&factory, &mut target, &egl, &surface),
        Instant::now(),
    );
}

#[test]
fn dormant_to_prepared_with_failing_factory_marks_blocked_and_retries() {
    let mock: Rc<MockFactory> = Rc::new(MockFactory::new());
    let factory: Rc<dyn RenderTargetFactory> = mock.clone();
    mock.fail(1);

    let mut target: Option<RenderTarget> = None;
    let egl = StubEgl;
    let surface = StubSurface;

    let mut sm = LifecycleStateMachine::new();
    sm.on_event(LifecycleState::Prepared);
    let t0 = Instant::now();
    sm.apply(
        &mut ctx_no_target(&factory, &mut target, &egl, &surface),
        t0,
    );

    assert_eq!(sm.current(), LifecycleState::Dormant);
    assert_eq!(sm.target(), LifecycleState::Prepared);
    assert!(sm.blocked());
    assert!(target.is_none());
    assert!(!sm.needs_retry(t0));
    assert!(sm.needs_retry(t0 + Duration::from_secs(1)));
    assert_eq!(mock.alloc_calls.get(), 1);
    assert_eq!(mock.destroy_calls.get(), 0);
}

#[test]
fn dormant_to_entering_with_failing_factory_marks_blocked_and_retries() {
    let mock: Rc<MockFactory> = Rc::new(MockFactory::new());
    let factory: Rc<dyn RenderTargetFactory> = mock.clone();
    mock.fail(1);

    let mut target: Option<RenderTarget> = None;
    let egl = StubEgl;
    let surface = StubSurface;

    let mut sm = LifecycleStateMachine::new();
    sm.on_event(LifecycleState::Entering);
    let t0 = Instant::now();
    sm.apply(
        &mut ctx_no_target(&factory, &mut target, &egl, &surface),
        t0,
    );

    assert_eq!(sm.current(), LifecycleState::Dormant);
    assert_eq!(sm.target(), LifecycleState::Entering);
    assert!(sm.blocked());
    assert!(target.is_none());
    assert!(!sm.needs_retry(t0));
    assert!(sm.needs_retry(t0 + Duration::from_secs(1)));
    assert_eq!(mock.alloc_calls.get(), 1);
}

#[test]
fn second_failure_resets_retry_timer() {
    let mock: Rc<MockFactory> = Rc::new(MockFactory::new());
    let factory: Rc<dyn RenderTargetFactory> = mock.clone();
    mock.fail(2);

    let mut target: Option<RenderTarget> = None;
    let egl = StubEgl;
    let surface = StubSurface;

    let mut sm = LifecycleStateMachine::new();
    sm.on_event(LifecycleState::Entering);
    let t0 = Instant::now();
    sm.apply(
        &mut ctx_no_target(&factory, &mut target, &egl, &surface),
        t0,
    );
    let first_retry = sm
        .retry_at()
        .expect("BUG: blocked after first failure must set retry_at");

    let t1 = t0 + Duration::from_secs(1) + Duration::from_millis(10);
    sm.apply(
        &mut ctx_no_target(&factory, &mut target, &egl, &surface),
        t1,
    );
    let second_retry = sm
        .retry_at()
        .expect("BUG: still blocked after second failure must keep retry_at populated");

    assert!(second_retry > first_retry);
    assert_eq!(mock.alloc_calls.get(), 2);
}

#[test]
fn apply_within_retry_window_does_not_call_factory() {
    // Locks in the retry-timer gate at the top of `apply`: while blocked and the timer
    // has not yet fired, repeated apply() calls (the main loop ticks every iteration)
    // must NOT call factory.allocate. Without the gate, a CMA-starved slot would hammer
    // the factory thousands of times per second.
    let mock: Rc<MockFactory> = Rc::new(MockFactory::new());
    let factory: Rc<dyn RenderTargetFactory> = mock.clone();
    mock.fail(10);

    let mut target: Option<RenderTarget> = None;
    let egl = StubEgl;
    let surface = StubSurface;

    let mut sm = LifecycleStateMachine::new();
    sm.on_event(LifecycleState::Entering);
    let t0 = Instant::now();
    sm.apply(
        &mut ctx_no_target(&factory, &mut target, &egl, &surface),
        t0,
    );
    assert_eq!(mock.alloc_calls.get(), 1);

    // Tick the main loop's `apply` ten more times within the 1 s retry window. None of
    // these must touch the factory.
    for step_ms in [1, 5, 50, 100, 200, 400, 600, 800, 900, 999] {
        sm.apply(
            &mut ctx_no_target(&factory, &mut target, &egl, &surface),
            t0 + Duration::from_millis(step_ms),
        );
        assert_eq!(
            mock.alloc_calls.get(),
            1,
            "retry timer breached at +{step_ms} ms",
        );
    }

    // Once the timer has elapsed, the next apply IS allowed to retry.
    sm.apply(
        &mut ctx_no_target(&factory, &mut target, &egl, &surface),
        t0 + Duration::from_millis(1_001),
    );
    assert_eq!(mock.alloc_calls.get(), 2);
}

#[test]
fn entering_to_dormant_while_blocked_clears_blocked_and_does_not_call_destroy() {
    let mock: Rc<MockFactory> = Rc::new(MockFactory::new());
    let factory: Rc<dyn RenderTargetFactory> = mock.clone();
    mock.fail(99);

    let mut target: Option<RenderTarget> = None;
    let egl = StubEgl;
    let surface = StubSurface;

    let mut sm = LifecycleStateMachine::new();
    sm.on_event(LifecycleState::Entering);
    sm.apply(
        &mut ctx_no_target(&factory, &mut target, &egl, &surface),
        Instant::now(),
    );
    assert!(sm.blocked());

    sm.on_event(LifecycleState::Dormant);
    sm.apply(
        &mut ctx_no_target(&factory, &mut target, &egl, &surface),
        Instant::now(),
    );

    assert_eq!(sm.current(), LifecycleState::Dormant);
    assert!(!sm.blocked());
    assert!(target.is_none());
    assert_eq!(mock.destroy_calls.get(), 0);
}

// Parametric coverage for the spec's 5x5 transition matrix. Each starting state in the
// buffer-free state Dormant is driven toward every target-owning state
// {Prepared, Entering, Visible, Leaving}; we use a factory configured to always fail so the post-
// transition state is observable without needing a real RenderTarget.
#[test]
fn all_transitions_from_dormant_to_target_owning_states_invoke_allocate() {
    use LifecycleState::{Dormant, Entering, Leaving, Prepared, Visible};
    let targets = [Prepared, Entering, Visible, Leaving];

    for target_state in targets {
        let mock: Rc<MockFactory> = Rc::new(MockFactory::new());
        let factory: Rc<dyn RenderTargetFactory> = mock.clone();
        mock.fail(1);

        let mut target: Option<RenderTarget> = None;
        let egl = StubEgl;
        let surface = StubSurface;

        let mut sm = LifecycleStateMachine::new();
        sm.on_event(target_state);
        sm.apply(
            &mut ctx_no_target(&factory, &mut target, &egl, &surface),
            Instant::now(),
        );

        assert_eq!(
            mock.alloc_calls.get(),
            1,
            "{Dormant:?} -> {target_state:?} must call allocate"
        );
        assert!(
            sm.blocked(),
            "{Dormant:?} -> {target_state:?} with failing factory must mark blocked"
        );
        assert_eq!(
            sm.current(),
            Dormant,
            "current must stay until allocation succeeds"
        );
        assert_eq!(sm.target(), target_state);
        assert!(target.is_none());
        assert_eq!(mock.destroy_calls.get(), 0);
    }
}

// Dormant self-transitions do not require any target resources.
#[test]
fn dormant_self_transition_is_no_op() {
    let mock: Rc<MockFactory> = Rc::new(MockFactory::new());
    let factory: Rc<dyn RenderTargetFactory> = mock.clone();
    let mut target: Option<RenderTarget> = None;
    let egl = StubEgl;
    let surface = StubSurface;

    let mut sm = LifecycleStateMachine::new();
    sm.on_event(LifecycleState::Dormant);
    sm.apply(
        &mut ctx_no_target(&factory, &mut target, &egl, &surface),
        Instant::now(),
    );

    assert_eq!(sm.current(), LifecycleState::Dormant);
    assert!(!sm.blocked());
    assert_eq!(mock.alloc_calls.get(), 0);
    assert_eq!(mock.destroy_calls.get(), 0);
}

#[test]
fn prepared_lifecycle_event_requests_surface_render() {
    let mut sm = LifecycleStateMachine::new();
    let effect = sm.on_event(LifecycleState::Prepared);

    assert!(effect.request_render);
}
