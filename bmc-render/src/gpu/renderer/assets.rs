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

//! FemtoVG's side of [`crate::renderer::RenderAssets`]: SVG, bitmap and mesh registries.

#![expect(clippy::cast_precision_loss)]

use super::FemtoVgRenderer;
use super::SphereRendererState;
use crate::gpu::mesh::{MeshDrawArgs, MeshRenderer, reserve_mesh};
use crate::gpu::sphere::SphereRenderer;
use crate::renderer::SvgContent;
use crate::renderer::prelude::*;
use crate::renderer::{AssetSuspendResult, AssetTagState};
use bmc_ui::colors::Color;
use bmc_ui::{BitmapId, MeshId, SvgId};

impl RenderAssets for FemtoVgRenderer {
    // -- Icons --

    fn register_svg(&mut self, tag: &str, data: &[u8]) -> Option<SvgId> {
        self.icon_registry.register(tag, data)
    }

    fn reserve_svg(&mut self, tag: &str) -> Option<SvgId> {
        self.icon_registry.reserve(tag)
    }

    fn suspend_svg(&mut self, tag: &str) -> AssetSuspendResult<SvgId> {
        self.icon_registry.suspend_exact(tag)
    }

    fn svg_tag_state(&self, tag: &str) -> AssetTagState<SvgId> {
        self.icon_registry.tag_state(tag)
    }

    fn svg_content(&self, icon_id: SvgId) -> Option<SvgContent> {
        self.icon_registry
            .get(icon_id)
            .and_then(|icon| icon.content)
    }

    fn draw_svg(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Color,
        icon_id: SvgId,
        anti_alias: bool,
        fills: &[(String, Color)],
    ) {
        if let Some(icon) = self.icon_registry.get(icon_id) {
            crate::gpu::svg::draw_svg(
                &mut self.canvas,
                icon,
                x,
                y,
                w,
                h,
                color,
                anti_alias,
                fills,
                self.brightness,
            );
        }
    }

    // -- Bitmaps --

    fn register_bitmap(&mut self, tag: &str, data: &[u8]) -> Option<BitmapId> {
        self.bitmap_registry
            .register(tag, data, &mut self.canvas, femtovg::ImageFlags::empty())
    }

    fn register_bitmap_nearest(&mut self, tag: &str, data: &[u8]) -> Option<BitmapId> {
        self.bitmap_registry
            .register(tag, data, &mut self.canvas, femtovg::ImageFlags::NEAREST)
    }

    fn reserve_bitmap(&mut self, tag: &str) -> Option<BitmapId> {
        self.bitmap_registry
            .reserve(tag, femtovg::ImageFlags::empty())
    }

    fn reserve_bitmap_nearest(&mut self, tag: &str) -> Option<BitmapId> {
        self.bitmap_registry
            .reserve(tag, femtovg::ImageFlags::NEAREST)
    }

    fn suspend_bitmap(&mut self, tag: &str) -> AssetSuspendResult<BitmapId> {
        let result = self.bitmap_registry.suspend_exact(tag, &mut self.canvas);
        if let AssetSuspendResult::Suspended(id) = result
            && self.sphere_bitmap_id == Some(id)
        {
            self.sphere_bitmap_id = None;
        }
        result
    }

    fn register_bitmap_rgba(
        &mut self,
        tag: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<BitmapId> {
        let bitmap_id = self.bitmap_registry.register_rgba(
            tag,
            rgba,
            width,
            height,
            &mut self.canvas,
            femtovg::ImageFlags::empty(),
        )?;
        if self.sphere_bitmap_id == Some(bitmap_id) {
            self.sphere_bitmap_id = None;
        }
        Some(bitmap_id)
    }

    fn register_bitmap_rgba_nearest(
        &mut self,
        tag: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<BitmapId> {
        let bitmap_id = self.bitmap_registry.register_rgba(
            tag,
            rgba,
            width,
            height,
            &mut self.canvas,
            femtovg::ImageFlags::NEAREST,
        )?;
        if self.sphere_bitmap_id == Some(bitmap_id) {
            self.sphere_bitmap_id = None;
        }
        Some(bitmap_id)
    }

    fn bitmap_tag_state(&self, tag: &str) -> AssetTagState<BitmapId> {
        self.bitmap_registry.tag_state(tag)
    }

    fn draw_bitmap(&mut self, x: f32, y: f32, w: f32, h: f32, bitmap_id: BitmapId) {
        let tint = self.image_tint();
        if let Some(image_id) = self.bitmap_registry.get(bitmap_id) {
            crate::gpu::bitmap::draw_bitmap(&mut self.canvas, image_id, x, y, w, h, tint);
        }
    }

    fn draw_nine_patch(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: BitmapId,
        left: u16,
        top: u16,
        right: u16,
        bottom: u16,
    ) {
        let tint = self.image_tint();
        if let Some((image_id, src_w, src_h)) = self.bitmap_registry.get_with_size(bitmap_id) {
            crate::gpu::bitmap::draw_nine_patch(
                &mut self.canvas,
                image_id,
                src_w as f32,
                src_h as f32,
                x,
                y,
                w,
                h,
                f32::from(left),
                f32::from(top),
                f32::from(right),
                f32::from(bottom),
                tint,
            );
        }
    }

    // -- Meshes --

    fn register_mesh(&mut self, tag: &str, data: &[u8]) -> Option<MeshId> {
        if !matches!(self.mesh_tag_state(tag), AssetTagState::Resident(_)) {
            crate::gpu_access::assert_gpu_access_authorized();
        }
        self.lazy_init_mesh_renderer();
        let renderer = self.mesh_renderer.as_mut()?;
        renderer.register_mesh(&self.gl, tag, data)
    }

    fn reserve_mesh(&mut self, tag: &str) -> Option<MeshId> {
        if let Some(renderer) = self.mesh_renderer.as_mut() {
            renderer.reserve(tag)
        } else {
            reserve_mesh(&mut self.pending_mesh_reservations, tag)
        }
    }

    fn suspend_mesh(&mut self, tag: &str) -> AssetSuspendResult<MeshId> {
        if matches!(self.mesh_tag_state(tag), AssetTagState::Resident(_)) {
            crate::gpu_access::assert_gpu_access_authorized();
        }
        if let Some(renderer) = self.mesh_renderer.as_mut() {
            renderer.suspend_exact(&self.gl, tag)
        } else {
            self.pending_mesh_reservations.suspend(tag).result()
        }
    }

    fn mesh_tag_state(&self, tag: &str) -> AssetTagState<MeshId> {
        if let Some(renderer) = self.mesh_renderer.as_ref() {
            renderer.tag_state(tag)
        } else {
            self.pending_mesh_reservations.tag_state(tag)
        }
    }

    fn draw_mesh(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        slot_index: u8,
        mesh_id: MeshId,
        args: MeshDrawArgs,
    ) {
        let tint = self.image_tint();
        self.lazy_init_mesh_renderer();
        let Some(renderer) = self.mesh_renderer.as_mut() else {
            return;
        };

        // Render mesh to atlas slot (skips if params unchanged).
        // `None` means out-of-range slot or evicted mesh
        //  — skip the draw so we don't sample stale slot pixels.
        let Some((image_id, sx, sy, sw, sh)) =
            renderer.render(&self.gl, slot_index, mesh_id, &args)
        else {
            return;
        };

        // Draw the atlas sub-rect via femtovg
        let (atlas_w, atlas_h) = renderer.atlas_size();
        crate::gpu::bitmap::draw_bitmap_subrect(
            &mut self.canvas,
            image_id,
            atlas_w,
            atlas_h,
            sx,
            sy,
            sw,
            sh,
            x,
            y,
            w,
            h,
            tint,
        );
    }

    // -- Sphere --

    #[expect(clippy::many_single_char_names)]
    fn draw_sphere(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bitmap_id: BitmapId,
        center_lat: f32,
        center_lon: f32,
        zoom: f32,
        light_lat: f32,
        light_lon: f32,
        atmosphere: bool,
    ) {
        let tint = self.image_tint();
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (width, height) = (w as u32, h as u32);
        if width == 0 || height == 0 {
            return;
        }

        if matches!(&self.sphere, SphereRendererState::Failed) {
            self.draw_bitmap(x, y, w, h, bitmap_id);
            return;
        }

        // Lazy-init sphere renderer on first call
        if matches!(&self.sphere, SphereRendererState::Uninitialized) {
            match SphereRenderer::new(&self.gl, &mut self.canvas, width, height) {
                Ok(sphere) => self.sphere = SphereRendererState::Ready(sphere),
                Err(e) => {
                    self.sphere = SphereRendererState::Failed;
                    tracing::error!("sphere init failed: {e}");
                    self.draw_bitmap(x, y, w, h, bitmap_id);
                    return;
                }
            }
        }
        let SphereRendererState::Ready(sphere) = &mut self.sphere else {
            unreachable!("BUG: sphere is initialized or returned above");
        };

        // Resolve the bitmap each frame so an evict+re-register cycle is
        // observed before we touch GL. Skip the draw on registry miss so a
        // stale FBO never gets sampled.
        let Some(image_id) = self.bitmap_registry.get(bitmap_id) else {
            self.sphere_bitmap_id = None;
            return;
        };

        // RGBA replacement, exact suspension, and eviction clear the binding,
        // so an equal BitmapId still refers to the bound native texture.
        if self.sphere_bitmap_id != Some(bitmap_id) {
            let Ok(tex) = self.canvas.get_native_texture(image_id) else {
                self.sphere_bitmap_id = None;
                return;
            };
            sphere.set_texture(tex);
            self.sphere_bitmap_id = Some(bitmap_id);
        }

        // When light is NaN, pass zero-vector to disable shading
        let (sl, sn) = if light_lat.is_nan() {
            (0.0, 0.0)
        } else {
            (light_lat, light_lon)
        };

        // Render sphere to offscreen FBO (skips if params unchanged). `None`
        // means no texture has been bound yet — skip the draw so we don't
        // sample stale FBO pixels.
        let Some(image_id) =
            sphere.render(&self.gl, center_lat, center_lon, zoom, sl, sn, atmosphere)
        else {
            return;
        };

        // Draw the FBO texture via femtovg
        crate::gpu::bitmap::draw_bitmap(&mut self.canvas, image_id, x, y, w, h, tint);
    }

    // -- Eviction --

    fn evict_prefix(&mut self, prefix: &str) -> usize {
        let mut n = self.icon_registry.evict_prefix(prefix);
        n += self.bitmap_registry.evict_prefix(prefix, &mut self.canvas);
        if let Some(mesh) = self.mesh_renderer.as_mut() {
            n += mesh.evict_prefix(&self.gl, prefix);
        } else {
            n += self.pending_mesh_reservations.evict_prefix(prefix).len();
        }
        if self
            .sphere_bitmap_id
            .is_some_and(|id| self.bitmap_registry.get(id).is_none())
        {
            self.sphere_bitmap_id = None;
        }
        // Static layers share the namespace, so a departing slot reclaims its
        // layer along with its icons, bitmaps and meshes. Deliberately not
        // counted: the total reports what the guest's own eviction removed, and
        // a layer is renderer scratch the guest never registered.
        let canvas = &mut self.canvas;
        self.static_layers.retain(|key, layer| {
            if key.starts_with(prefix) {
                canvas.delete_image(layer.image);
                false
            } else {
                true
            }
        });
        self.incomplete_static_layers
            .retain(|key, _| !key.starts_with(prefix));
        n
    }

    fn evict_prefix_requires_gpu_access(&self, prefix: &str) -> bool {
        self.bitmap_registry.has_resident_prefix(prefix)
            || self
                .mesh_renderer
                .as_ref()
                .is_some_and(|mesh| mesh.has_resident_prefix(prefix))
            // Reclaiming a layer deletes a femtovg image, so the eviction needs
            // the GPU lock.
            || self
                .static_layers
                .keys()
                .any(|key| key.starts_with(prefix))
    }

    fn bitmap_resident_bytes(&self) -> u64 {
        self.bitmap_registry.resident_bytes()
    }

    fn svg_resident_path_bytes(&self) -> u64 {
        self.icon_registry.resident_path_bytes()
    }

    fn mesh_resident_bytes(&self) -> u64 {
        self.mesh_renderer
            .as_ref()
            .map_or(0, MeshRenderer::resident_bytes)
    }
}

impl FemtoVgRenderer {
    /// Lazy-initialise the mesh renderer on first use. Logs and leaves
    /// `self.mesh_renderer` as `None` if creation fails — callers must
    /// observe the `None` and bail out gracefully rather than relying on
    /// "init succeeded just now" invariants.
    fn lazy_init_mesh_renderer(&mut self) {
        if self.mesh_renderer.is_some() {
            return;
        }
        match MeshRenderer::new_with_reservations(
            &self.gl,
            &mut self.canvas,
            self.mesh_msaa_samples,
            &mut self.pending_mesh_reservations,
        ) {
            Ok(r) => self.mesh_renderer = Some(r),
            Err(e) => tracing::error!("mesh renderer init failed: {e}"),
        }
    }
}
