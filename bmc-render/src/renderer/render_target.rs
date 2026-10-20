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

//! The [`Renderer`] that tree walks draw through, resolving suspended assets on first use.

use super::AssetSuspendResult;
use super::AssetTagState;
use super::FrameClear;
use super::GlyphCacheCounters;
use super::RendererAssetResolver;
use super::SvgContent;
use super::TextLayoutCounters;
use crate::gpu::mesh::MeshDrawArgs;
use crate::interaction::Rect;
use crate::renderer::prelude::*;
use crate::tree::{AutoFit, SpanData, TextStyle};
use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{
    ArcAnchor, ArcCap, ArcFill, ArcSegments, ArcTextFacing, BitmapId, Fill, MeshId, SvgId,
};
use std::cell::RefCell;

pub(crate) struct RenderTarget<'renderer, 'cell, 'resolver> {
    renderer: &'renderer mut dyn Renderer,
    resolver: Option<&'cell RefCell<&'resolver mut dyn RendererAssetResolver>>,
}

impl<'renderer, 'cell, 'resolver> RenderTarget<'renderer, 'cell, 'resolver> {
    pub(crate) fn new(
        renderer: &'renderer mut dyn Renderer,
        resolver: Option<&'cell RefCell<&'resolver mut dyn RendererAssetResolver>>,
    ) -> Self {
        Self { renderer, resolver }
    }

    #[expect(clippy::too_many_arguments)]
    pub(crate) fn drop_shadow(
        &mut self,
        cx: f32,
        cy: f32,
        fbo_w: u32,
        fbo_h: u32,
        dx: f32,
        dy: f32,
        blur: f32,
        color: Color,
        inner: &mut dyn FnMut(&mut RenderTarget<'_, '_, '_>),
    ) {
        let resolver = self.resolver;
        self.renderer
            .drop_shadow(cx, cy, fbo_w, fbo_h, dx, dy, blur, color, &mut |renderer| {
                let mut target = RenderTarget::new(renderer, resolver);
                inner(&mut target);
            });
    }
}

impl RenderFrame for RenderTarget<'_, '_, '_> {
    fn save(&mut self) {
        self.renderer.save();
    }

    fn restore(&mut self) {
        self.renderer.restore();
    }

    fn translate(&mut self, x: f32, y: f32) {
        self.renderer.translate(x, y);
    }

    fn rotate(&mut self, angle_radians: f32) {
        self.renderer.rotate(angle_radians);
    }

    fn push_scissor(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.renderer.push_scissor(x, y, w, h);
    }

    fn pop_scissor(&mut self) {
        self.renderer.pop_scissor();
    }

    fn brightness(&self) -> f32 {
        self.renderer.brightness()
    }

    fn set_brightness(&mut self, brightness: f32) {
        self.renderer.set_brightness(brightness);
    }

    fn begin_frame(&mut self, width: u32, height: u32, dpi_scale: f32) {
        self.renderer.begin_frame(width, height, dpi_scale);
    }

    fn begin_frame_with_clear(
        &mut self,
        width: u32,
        height: u32,
        dpi_scale: f32,
        clear: FrameClear,
    ) {
        self.renderer
            .begin_frame_with_clear(width, height, dpi_scale, clear);
    }

    fn flush(&mut self) {
        self.renderer.flush();
    }

    fn width(&self) -> f32 {
        self.renderer.width()
    }

    fn height(&self) -> f32 {
        self.renderer.height()
    }

    fn flush_and_fence_us(&mut self) -> u32 {
        self.renderer.flush_and_fence_us()
    }
}

impl RenderShapes for RenderTarget<'_, '_, '_> {
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        self.renderer.fill_rect(x, y, w, h, color);
    }

    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color) {
        self.renderer.fill_rounded_rect(x, y, w, h, radius, color);
    }

    fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        self.renderer.fill_circle(cx, cy, r, color);
    }

    fn fill_rect_paint(&mut self, x: f32, y: f32, w: f32, h: f32, fill: &Fill) {
        self.renderer.fill_rect_paint(x, y, w, h, fill);
    }

    fn fill_circle_paint(&mut self, cx: f32, cy: f32, r: f32, fill: &Fill) {
        self.renderer.fill_circle_paint(cx, cy, r, fill);
    }

    fn stroke_arc(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        start_angle: f32,
        end_angle: f32,
        width: f32,
        fill: &ArcFill,
        segments: &ArcSegments,
        cap: ArcCap,
    ) {
        self.renderer.stroke_arc(
            cx,
            cy,
            radius,
            start_angle,
            end_angle,
            width,
            fill,
            segments,
            cap,
        );
    }

    fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, border_width: f32, color: Color) {
        self.renderer.stroke_rect(x, y, w, h, border_width, color);
    }

    fn stroke_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        border_width: f32,
        color: Color,
    ) {
        self.renderer
            .stroke_rounded_rect(x, y, w, h, radius, border_width, color);
    }

    fn draw_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: Color) {
        self.renderer.draw_line(x1, y1, x2, y2, width, color);
    }

    fn stroke_path(
        &mut self,
        points: &[(f32, f32)],
        stroke_width: f32,
        color: Color,
        closed: bool,
        smooth: bool,
    ) {
        self.renderer
            .stroke_path(points, stroke_width, color, closed, smooth);
    }

    fn fill_path_paint(&mut self, points: &[(f32, f32)], fill: &Fill, smooth: bool) {
        self.renderer.fill_path_paint(points, fill, smooth);
    }
}

impl RenderText for RenderTarget<'_, '_, '_> {
    fn draw_text(&mut self, text: &str, x: f32, y: f32, size: f32, color: Color) {
        self.renderer.draw_text(text, x, y, size, color);
    }

    fn measure_text(&mut self, text: &str, size: f32) -> f32 {
        self.renderer.measure_text(text, size)
    }

    fn measure_paragraph(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        max_width: Option<f32>,
    ) -> (f32, f32) {
        self.renderer.measure_paragraph(style, spans, max_width)
    }

    fn draw_paragraph(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        x: f32,
        y: f32,
        max_width: f32,
    ) {
        self.renderer.draw_paragraph(style, spans, x, y, max_width);
    }

    fn draw_paragraph_clipped(
        &mut self,
        style: &TextStyle,
        spans: &[SpanData],
        x: f32,
        y: f32,
        max_width: f32,
        clip_top: f32,
        clip_bottom: f32,
    ) {
        self.renderer
            .draw_paragraph_clipped(style, spans, x, y, max_width, clip_top, clip_bottom);
    }

    fn draw_canvas_text(&mut self, text: &str, x: f32, y: f32, style: &TextStyle) {
        self.renderer.draw_canvas_text(text, x, y, style);
    }

    fn draw_autofit_text(
        &mut self,
        x: f32,
        y: f32,
        box_width: f32,
        box_height: f32,
        text: &str,
        style: &TextStyle,
        mode: AutoFit,
        min_size: u16,
        max_size: u16,
    ) {
        self.renderer.draw_autofit_text(
            x, y, box_width, box_height, text, style, mode, min_size, max_size,
        );
    }

    fn draw_curved_text(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        angle: f32,
        anchor: ArcAnchor,
        facing: ArcTextFacing,
        text: &str,
        style: &TextStyle,
    ) {
        self.renderer
            .draw_curved_text(cx, cy, radius, angle, anchor, facing, text, style);
    }

    fn text_atlas_resident_bytes(&self) -> u64 {
        self.renderer.text_atlas_resident_bytes()
    }

    fn glyph_cache_counters(&self) -> GlyphCacheCounters {
        self.renderer.glyph_cache_counters()
    }

    fn glyph_cache_metadata_capacity_bytes(&self) -> u64 {
        self.renderer.glyph_cache_metadata_capacity_bytes()
    }

    fn text_layout_counters(&self) -> TextLayoutCounters {
        self.renderer.text_layout_counters()
    }

    fn paragraph_cache_stats(&self) -> (u32, usize) {
        self.renderer.paragraph_cache_stats()
    }
}

impl RenderAssets for RenderTarget<'_, '_, '_> {
    fn register_svg(&mut self, tag: &str, data: &[u8]) -> Option<SvgId> {
        self.renderer.register_svg(tag, data)
    }

    fn reserve_svg(&mut self, tag: &str) -> Option<SvgId> {
        self.renderer.reserve_svg(tag)
    }

    fn suspend_svg(&mut self, tag: &str) -> AssetSuspendResult<SvgId> {
        self.renderer.suspend_svg(tag)
    }

    fn svg_tag_state(&self, tag: &str) -> AssetTagState<SvgId> {
        self.renderer.svg_tag_state(tag)
    }

    fn svg_content(&self, icon_id: SvgId) -> Option<SvgContent> {
        self.renderer.svg_content(icon_id)
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
        if self
            .resolver
            .is_none_or(|resolver| resolver.borrow_mut().resolve_svg(self.renderer, icon_id))
        {
            self.renderer
                .draw_svg(x, y, w, h, color, icon_id, anti_alias, fills);
        }
    }

    fn register_bitmap(&mut self, tag: &str, data: &[u8]) -> Option<BitmapId> {
        self.renderer.register_bitmap(tag, data)
    }

    fn register_bitmap_nearest(&mut self, tag: &str, data: &[u8]) -> Option<BitmapId> {
        self.renderer.register_bitmap_nearest(tag, data)
    }

    fn reserve_bitmap(&mut self, tag: &str) -> Option<BitmapId> {
        self.renderer.reserve_bitmap(tag)
    }

    fn reserve_bitmap_nearest(&mut self, tag: &str) -> Option<BitmapId> {
        self.renderer.reserve_bitmap_nearest(tag)
    }

    fn suspend_bitmap(&mut self, tag: &str) -> AssetSuspendResult<BitmapId> {
        self.renderer.suspend_bitmap(tag)
    }

    fn register_bitmap_rgba(
        &mut self,
        tag: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<BitmapId> {
        self.renderer.register_bitmap_rgba(tag, rgba, width, height)
    }

    fn register_bitmap_rgba_nearest(
        &mut self,
        tag: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Option<BitmapId> {
        self.renderer
            .register_bitmap_rgba_nearest(tag, rgba, width, height)
    }

    fn bitmap_tag_state(&self, tag: &str) -> AssetTagState<BitmapId> {
        self.renderer.bitmap_tag_state(tag)
    }

    fn draw_bitmap(&mut self, x: f32, y: f32, w: f32, h: f32, bitmap_id: BitmapId) {
        if self.resolver.is_none_or(|resolver| {
            resolver
                .borrow_mut()
                .resolve_bitmap(self.renderer, bitmap_id)
        }) {
            self.renderer.draw_bitmap(x, y, w, h, bitmap_id);
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
        if self.resolver.is_none_or(|resolver| {
            resolver
                .borrow_mut()
                .resolve_bitmap(self.renderer, bitmap_id)
        }) {
            self.renderer
                .draw_nine_patch(x, y, w, h, bitmap_id, left, top, right, bottom);
        }
    }

    fn register_mesh(&mut self, tag: &str, data: &[u8]) -> Option<MeshId> {
        self.renderer.register_mesh(tag, data)
    }

    fn reserve_mesh(&mut self, tag: &str) -> Option<MeshId> {
        self.renderer.reserve_mesh(tag)
    }

    fn suspend_mesh(&mut self, tag: &str) -> AssetSuspendResult<MeshId> {
        self.renderer.suspend_mesh(tag)
    }

    fn mesh_tag_state(&self, tag: &str) -> AssetTagState<MeshId> {
        self.renderer.mesh_tag_state(tag)
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
        if self
            .resolver
            .is_none_or(|resolver| resolver.borrow_mut().resolve_mesh(self.renderer, mesh_id))
        {
            self.renderer
                .draw_mesh(x, y, w, h, slot_index, mesh_id, args);
        }
    }

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
        if self.resolver.is_none_or(|resolver| {
            resolver
                .borrow_mut()
                .resolve_bitmap(self.renderer, bitmap_id)
        }) {
            self.renderer.draw_sphere(
                x, y, w, h, bitmap_id, center_lat, center_lon, zoom, light_lat, light_lon,
                atmosphere,
            );
        }
    }

    fn evict_prefix(&mut self, prefix: &str) -> usize {
        self.renderer.evict_prefix(prefix)
    }

    fn evict_prefix_requires_gpu_access(&self, prefix: &str) -> bool {
        self.renderer.evict_prefix_requires_gpu_access(prefix)
    }

    fn bitmap_resident_bytes(&self) -> u64 {
        self.renderer.bitmap_resident_bytes()
    }

    fn svg_resident_path_bytes(&self) -> u64 {
        self.renderer.svg_resident_path_bytes()
    }

    fn mesh_resident_bytes(&self) -> u64 {
        self.renderer.mesh_resident_bytes()
    }
}

impl RenderOffscreen for RenderTarget<'_, '_, '_> {
    fn drop_shadow(
        &mut self,
        cx: f32,
        cy: f32,
        fbo_w: u32,
        fbo_h: u32,
        dx: f32,
        dy: f32,
        blur: f32,
        color: Color,
        inner: &mut dyn FnMut(&mut dyn Renderer),
    ) {
        let resolver = self.resolver;
        self.renderer
            .drop_shadow(cx, cy, fbo_w, fbo_h, dx, dy, blur, color, &mut |renderer| {
                let mut target = RenderTarget::new(renderer, resolver);
                inner(&mut target);
            });
    }

    // Forwarded, not defaulted: the inert defaults would have a wrapped renderer
    // report "no layer available", indistinguishable from one that genuinely
    // cannot render to a texture.
    fn begin_static_layer(&mut self, key: &str, width: u32, height: u32) -> bool {
        self.renderer.begin_static_layer(key, width, height)
    }

    fn end_static_layer(&mut self, key: &str) {
        self.renderer.end_static_layer(key);
    }

    fn blit_static_layer(&mut self, key: &str) -> bool {
        self.renderer.blit_static_layer(key)
    }

    fn blit_static_layer_rects(&mut self, key: &str, rects: &[Rect]) -> bool {
        self.renderer.blit_static_layer_rects(key, rects)
    }

    fn invalidate_static_layer(&mut self, key: &str) {
        self.renderer.invalidate_static_layer(key);
    }
}
