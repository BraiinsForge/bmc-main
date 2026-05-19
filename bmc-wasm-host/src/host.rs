// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_render::gpu::FemtoVgRenderer;
use bmc_widget::egl::{EglContext, SharedRenderScratch};

/// State shared across all widget slots within a host.
///
/// Aliasing invariant: `SharedHost` does NOT own the `Renderer`. The
/// renderer is owned by `main_loop::run` and parked behind a `NonNull` for
/// host imports to reborrow. Adding a `Renderer` (or anything containing
/// one) here would alias the parked pointer when the main loop passes
/// `&mut shared` and the parked `NonNull<dyn Renderer>` to the same call,
/// producing UB.
#[expect(missing_debug_implementations)]
pub struct SharedHost {
    pub egl: EglContext,
    pub scratch: SharedRenderScratch,
    pub font_cache: FontCache,
}

#[derive(Debug, Default)]
pub struct FontCache;

impl SharedHost {
    pub fn init(display_max_w: u32, display_max_h: u32) -> anyhow::Result<(Self, FemtoVgRenderer)> {
        tracing::info!(
            display_max_w,
            display_max_h,
            "initializing shared wasm host renderer"
        );
        let egl = EglContext::new()?;
        let scratch = SharedRenderScratch::new(&egl, display_max_w, display_max_h)?;
        let renderer = unsafe {
            FemtoVgRenderer::new(
                |sym: &str| EglContext::get_proc_address(sym),
                display_max_w,
                display_max_h,
                scratch.staging_fbo_id(),
                0,
            )?
        };
        tracing::info!("shared wasm host renderer initialized");
        Ok((
            Self {
                egl,
                scratch,
                font_cache: FontCache,
            },
            renderer,
        ))
    }
}
