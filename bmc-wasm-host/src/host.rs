// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::HashMap;

use bmc_render::gpu::FemtoVgRenderer;
use bmc_widget::egl::{EglContext, SharedRenderScratch};
use femtovg::FontId;
use sha2::{Digest, Sha256};

#[expect(missing_debug_implementations)]
pub struct SharedHost {
    pub egl: EglContext,
    pub scratch: SharedRenderScratch,
    pub renderer: FemtoVgRenderer,
    pub font_cache: FontCache,
}

#[derive(Debug)]
pub struct FontCache {
    entries: HashMap<[u8; 32], FontId>,
    refcounts: HashMap<FontId, usize>,
}

impl FontCache {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            refcounts: HashMap::new(),
        }
    }

    pub fn register(
        &mut self,
        canvas: &mut femtovg::Canvas<impl femtovg::Renderer>,
        bytes: &[u8],
    ) -> Result<FontId, femtovg::ErrorKind> {
        let hash: [u8; 32] = Sha256::digest(bytes).into();
        if let Some(&id) = self.entries.get(&hash) {
            *self.refcounts.entry(id).or_insert(0) += 1;
            return Ok(id);
        }
        let id = canvas.add_font_mem(bytes)?;
        self.entries.insert(hash, id);
        self.refcounts.insert(id, 1);
        Ok(id)
    }

    pub fn release(&mut self, id: FontId) {
        if let Some(rc) = self.refcounts.get_mut(&id) {
            *rc = rc.saturating_sub(1);
        }
    }
}

impl SharedHost {
    pub fn init(display_max_w: u32, display_max_h: u32) -> anyhow::Result<Self> {
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
        Ok(Self {
            egl,
            scratch,
            renderer,
            font_cache: FontCache::new(),
        })
    }
}
