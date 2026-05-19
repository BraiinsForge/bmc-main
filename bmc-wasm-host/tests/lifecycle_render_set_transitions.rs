// Copyright (C) 2026  Braiins Systems s.r.o.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use bmc_wasm_host::lifecycle::{
    LifecycleEgl, LifecycleState, LifecycleStateMachine, LifecycleSurface, SlotApplyCtx,
};
use bmc_wasm_host::render_target::{RenderTarget, RenderTargetError, RenderTargetFactory};

struct StubEgl;
impl LifecycleEgl for StubEgl {
    fn as_egl_context(&self) -> &bmc_widget::egl::EglContext {
        unimplemented!("CountingFactory never calls as_egl_context in this fixture")
    }
}

struct StubSurface;
impl LifecycleSurface for StubSurface {
    fn as_deck_widget_surface(&self) -> &bmc_widget::surface::DeckWidgetSurfaceClient {
        unimplemented!("CountingFactory never calls as_deck_widget_surface in this fixture")
    }

    fn mint_wl_buffer(
        &mut self,
        _: &bmc_widget::egl::DmaBufInfo,
        _: usize,
    ) -> Result<wayland_client::protocol::wl_buffer::WlBuffer, String> {
        unimplemented!("CountingFactory never calls mint_wl_buffer in this fixture")
    }

    fn destroy_minted_wl_buffer(&mut self, _: wayland_client::protocol::wl_buffer::WlBuffer) {
        unimplemented!("CountingFactory never calls destroy_minted_wl_buffer in this fixture")
    }
}

struct CountingFactory {
    alloc_calls: Cell<u32>,
    destroy_calls: Cell<u32>,
}

impl CountingFactory {
    fn new() -> Self {
        Self {
            alloc_calls: Cell::new(0),
            destroy_calls: Cell::new(0),
        }
    }

    fn reset(&self) {
        self.alloc_calls.set(0);
        self.destroy_calls.set(0);
    }
}

impl RenderTargetFactory for CountingFactory {
    fn allocate(
        &self,
        _: &dyn LifecycleEgl,
        _: &mut dyn LifecycleSurface,
        width: u32,
        height: u32,
    ) -> Result<RenderTarget, RenderTargetError> {
        self.alloc_calls.set(self.alloc_calls.get() + 1);
        Ok(RenderTarget::new_stub(width, height))
    }

    fn destroy(&self, _: RenderTarget, _: &dyn LifecycleEgl, _: &mut dyn LifecycleSurface) {
        self.destroy_calls.set(self.destroy_calls.get() + 1);
    }
}

fn ctx<'a>(
    factory: &'a Rc<dyn RenderTargetFactory>,
    target: &'a mut Option<RenderTarget>,
    egl: &'a StubEgl,
    surface: &'a mut StubSurface,
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

fn seeded_target_owning_state(
    source: LifecycleState,
) -> (
    LifecycleStateMachine,
    Option<RenderTarget>,
    Rc<CountingFactory>,
    Rc<dyn RenderTargetFactory>,
) {
    let mock: Rc<CountingFactory> = Rc::new(CountingFactory::new());
    let factory: Rc<dyn RenderTargetFactory> = mock.clone();
    let mut target = None;
    let egl = StubEgl;
    let mut surface = StubSurface;

    let mut sm = LifecycleStateMachine::new();
    sm.on_event(source);
    sm.apply(
        &mut ctx(&factory, &mut target, &egl, &mut surface),
        Instant::now(),
    );

    assert_eq!(sm.current(), source);
    assert!(
        target.is_some(),
        "target-owning source must seed a render target"
    );
    assert_eq!(mock.alloc_calls.get(), 1);
    assert_eq!(mock.destroy_calls.get(), 0);
    mock.reset();

    (sm, target, mock, factory)
}

#[test]
fn target_owning_self_and_intra_set_transitions_do_not_churn_target() {
    use LifecycleState::{Entering, Leaving, Prepared, Visible};
    let target_owning_set = [Prepared, Entering, Visible, Leaving];

    for source in target_owning_set {
        for target_state in target_owning_set {
            let (mut sm, mut target, mock, factory) = seeded_target_owning_state(source);
            let egl = StubEgl;
            let mut surface = StubSurface;

            sm.on_event(target_state);
            sm.apply(
                &mut ctx(&factory, &mut target, &egl, &mut surface),
                Instant::now(),
            );

            assert_eq!(sm.current(), target_state);
            assert!(target.is_some());
            assert_eq!(mock.alloc_calls.get(), 0);
            assert_eq!(mock.destroy_calls.get(), 0);
        }
    }
}

#[test]
fn target_owning_to_dormant_transitions_destroy_exactly_once() {
    use LifecycleState::{Dormant, Entering, Leaving, Prepared, Visible};
    let target_owning_set = [Prepared, Entering, Visible, Leaving];

    for source in target_owning_set {
        let (mut sm, mut target, mock, factory) = seeded_target_owning_state(source);
        let egl = StubEgl;
        let mut surface = StubSurface;

        sm.on_event(Dormant);
        sm.apply(
            &mut ctx(&factory, &mut target, &egl, &mut surface),
            Instant::now(),
        );

        assert_eq!(sm.current(), Dormant);
        assert!(target.is_none());
        assert_eq!(mock.alloc_calls.get(), 0);
        assert_eq!(mock.destroy_calls.get(), 1);
    }
}
