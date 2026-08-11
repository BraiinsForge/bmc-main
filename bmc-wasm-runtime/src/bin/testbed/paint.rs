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

//! GL plumbing and non-egui-widget rendering helpers for the testbed:
//! per-tile FBO + texture pair, GL proc-address loader shim, checkerboard
//! backdrop, LED-strip rendering, the timing-chart + legend, and the
//! `--perf-report=` JSON writer.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::integer_division,
    reason = "UI / GL math on small bounded positive values, GL u32 enums cast to GLint"
)]

use std::ffi::CString;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use eframe::glow::HasContext as _;

use super::{LED_STRIP_H, PreviewTile, platforms::DisplayShape};

// ── GL helpers ──────────────────────────────────────────────────────

/// FBO + texture pair allocated against eframe's glow context. Each tile owns one;
/// `WasmWidgetRuntime` renders into the FBO and we paint the texture in egui.
pub(super) struct TileGpu {
    fbo: Option<eframe::glow::Framebuffer>,
    rbo: Option<eframe::glow::Renderbuffer>,
    /// Registered with eframe via `register_native_glow_texture`; eframe owns
    /// the native texture lifetime after registration. There is no public
    /// unregister for this native id, so switch cleanup deletes only the GL
    /// framebuffer/renderbuffer objects still owned by the testbed.
    texture: eframe::glow::Texture,
    pub(super) egui_tex_id: egui::TextureId,
    pub(super) width: u32,
    pub(super) height: u32,
}

impl TileGpu {
    /// Create an `width × height` RGBA8 colour texture + matching framebuffer.
    /// The texture is registered with egui's frame so callers paint it as an `egui::Image`
    /// after the underlying GL render finishes.
    pub(super) fn new(
        gl: &eframe::glow::Context,
        frame: &mut eframe::Frame,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        // SAFETY: eframe's glow context is current on the calling thread inside `App::ui`.
        unsafe {
            let texture = gl
                .create_texture()
                .map_err(|e| anyhow::anyhow!("create_texture: {e}"))?;
            configure_texture(gl, texture, width, height);
            let (fbo, rbo) = create_render_target(gl, texture, width, height)?;

            let native = eframe::glow::NativeTexture(texture.0);
            let egui_tex_id = frame.register_native_glow_texture(native);

            Ok(Self {
                fbo: Some(fbo),
                rbo: Some(rbo),
                texture,
                egui_tex_id,
                width,
                height,
            })
        }
    }

    /// Reuse this already-registered texture for a new tile size.
    ///
    /// `egui_tex_id` stays unchanged: eframe owns the native texture
    /// registration and exposes no public unregister/replace API. The testbed
    /// still owns the FBO/RBO that attach to that texture, so those are
    /// recreated whenever the pooled texture is resized for a new tile.
    pub(super) fn reinitialize(
        &mut self,
        gl: &eframe::glow::Context,
        width: u32,
        height: u32,
    ) -> Result<()> {
        self.destroy_render_target(gl);
        configure_texture(gl, self.texture, width, height);
        let (fbo, rbo) = create_render_target(gl, self.texture, width, height)?;
        self.fbo = Some(fbo);
        self.rbo = Some(rbo);
        self.width = width;
        self.height = height;
        Ok(())
    }

    /// Numeric FBO ID for `WasmWidgetRuntime::new(... fbo_id ...)`.
    pub(super) fn fbo_id(&self) -> u32 {
        self.fbo
            .expect("BUG: TileGpu framebuffer used after destroy")
            .0
            .get()
    }

    pub(super) fn detach_render_target(&mut self, gl: &eframe::glow::Context) {
        self.destroy_render_target(gl);
    }

    fn destroy_render_target(&mut self, gl: &eframe::glow::Context) {
        // SAFETY: called from egui's UI pass or from `reinitialize`, where
        // eframe keeps the glow context current.
        unsafe {
            if let Some(fbo) = self.fbo.take() {
                gl.delete_framebuffer(fbo);
            }
            if let Some(rbo) = self.rbo.take() {
                gl.delete_renderbuffer(rbo);
            }
        }
    }
}

fn configure_texture(
    gl: &eframe::glow::Context,
    texture: eframe::glow::Texture,
    width: u32,
    height: u32,
) {
    // SAFETY: callers run inside egui's UI pass, where eframe keeps the glow
    // context current.
    unsafe {
        gl.bind_texture(eframe::glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            eframe::glow::TEXTURE_2D,
            0,
            eframe::glow::RGBA8 as i32,
            width as i32,
            height as i32,
            0,
            eframe::glow::RGBA,
            eframe::glow::UNSIGNED_BYTE,
            eframe::glow::PixelUnpackData::Slice(None),
        );
        gl.tex_parameter_i32(
            eframe::glow::TEXTURE_2D,
            eframe::glow::TEXTURE_MIN_FILTER,
            eframe::glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            eframe::glow::TEXTURE_2D,
            eframe::glow::TEXTURE_MAG_FILTER,
            eframe::glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            eframe::glow::TEXTURE_2D,
            eframe::glow::TEXTURE_WRAP_S,
            eframe::glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            eframe::glow::TEXTURE_2D,
            eframe::glow::TEXTURE_WRAP_T,
            eframe::glow::CLAMP_TO_EDGE as i32,
        );
        gl.bind_texture(eframe::glow::TEXTURE_2D, None);
    }
}

fn create_render_target(
    gl: &eframe::glow::Context,
    texture: eframe::glow::Texture,
    width: u32,
    height: u32,
) -> Result<(eframe::glow::Framebuffer, eframe::glow::Renderbuffer)> {
    // SAFETY: callers run inside egui's UI pass, where eframe keeps the glow
    // context current.
    unsafe {
        let fbo = gl
            .create_framebuffer()
            .map_err(|e| anyhow::anyhow!("create_framebuffer: {e}"))?;
        gl.bind_framebuffer(eframe::glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            eframe::glow::FRAMEBUFFER,
            eframe::glow::COLOR_ATTACHMENT0,
            eframe::glow::TEXTURE_2D,
            Some(texture),
            0,
        );

        // Stencil renderbuffer — FemtoVG's stroke shader uses stencil.
        let rbo = gl
            .create_renderbuffer()
            .map_err(|e| anyhow::anyhow!("create_renderbuffer: {e}"))?;
        gl.bind_renderbuffer(eframe::glow::RENDERBUFFER, Some(rbo));
        gl.renderbuffer_storage(
            eframe::glow::RENDERBUFFER,
            eframe::glow::DEPTH24_STENCIL8,
            width as i32,
            height as i32,
        );
        gl.framebuffer_renderbuffer(
            eframe::glow::FRAMEBUFFER,
            eframe::glow::DEPTH_STENCIL_ATTACHMENT,
            eframe::glow::RENDERBUFFER,
            Some(rbo),
        );
        gl.bind_renderbuffer(eframe::glow::RENDERBUFFER, None);

        let status = gl.check_framebuffer_status(eframe::glow::FRAMEBUFFER);
        if status != eframe::glow::FRAMEBUFFER_COMPLETE {
            anyhow::bail!("FBO incomplete: {status:#x}");
        }
        gl.bind_framebuffer(eframe::glow::FRAMEBUFFER, None);

        Ok((fbo, rbo))
    }
}

/// Wraps `cc.get_proc_address` into the shape `WasmWidgetRuntime::new` accepts (a `&str`-keyed
/// loader). The eframe-provided callback takes `&CStr`; we allocate the `CString` per call
/// since the runtime constructor only runs once per widget construction.
pub(super) fn proc_loader(get_proc: GlProcAddress) -> impl FnMut(&str) -> *const std::ffi::c_void {
    move |name: &str| {
        let Ok(cstr) = CString::new(name) else {
            return std::ptr::null();
        };
        get_proc(&cstr)
    }
}

/// Eframe's GL function loader closure shape — `&CStr` → raw function pointer.
/// Aliased so the `dyn Fn` trait object isn't spelled out at every storage site.
pub(super) type GlProcAddress =
    Arc<dyn Fn(&std::ffi::CStr) -> *const std::ffi::c_void + Send + Sync>;

// ── Background ──────────────────────────────────────────────────────

/// Paint a two-tone checkerboard over `rect` — same pattern as `bmc-virt-console`'s
/// device backdrop so the tile boundaries read clearly against an otherwise blank window.
pub(super) fn draw_checkerboard(painter: &egui::Painter, rect: egui::Rect) {
    let size = 16.0;
    let color_a = egui::Color32::from_gray(24);
    let color_b = egui::Color32::from_gray(32);
    let cols = (rect.width() / size).ceil() as usize;
    let rows = (rect.height() / size).ceil() as usize;
    for row in 0..rows {
        for col in 0..cols {
            let color = if (row + col) % 2 == 0 {
                color_a
            } else {
                color_b
            };
            let pos = rect.min + egui::vec2(col as f32 * size, row as f32 * size);
            let cell_rect = egui::Rect::from_min_size(pos, egui::vec2(size, size));
            painter.rect_filled(cell_rect, 0.0, color);
        }
    }
}

// ── LED strip rendering (egui painter, gradient approximation) ──────

/// Brightness ∈ [0, 1] for LED `phase` (0..1) at time `anim_t`.
/// Ported from the prior FemtoVG-based strip with identical semantics.
fn led_brightness(
    effect: bmc_led::data::LedEffect,
    phase: f32,
    anim_t: f32,
    led_count: usize,
) -> f32 {
    use bmc_led::data::LedEffect;
    match &effect {
        LedEffect::Solid(_) => 1.0,
        LedEffect::Breathe(_) => {
            let pulse = f32::midpoint((anim_t * std::f32::consts::TAU).sin(), 1.0);
            0.3 + pulse * 0.7
        }
        LedEffect::Chase(_) => {
            let pos = anim_t.fract();
            let dist = (phase - pos)
                .abs()
                .min((phase - pos + 1.0).abs())
                .min((phase - pos - 1.0).abs());
            (1.0 - dist * led_count as f32 * 0.5).max(0.02)
        }
        LedEffect::KnightRider(_) | LedEffect::Scan(_) => {
            let pos = (anim_t.fract() * 2.0 - 1.0).abs();
            let dist = (phase - pos).abs();
            (1.0 - dist * led_count as f32 * 0.5).max(0.02)
        }
        LedEffect::Snake(_) => {
            let tail = (phase - anim_t.fract() + 1.0).fract();
            let tail_len = 0.3;
            if tail < tail_len {
                1.0 - tail / tail_len
            } else {
                0.02
            }
        }
        LedEffect::None => 0.0,
    }
}

/// Paint an LED diffuser strip below a tile.
///
/// Black background always; glow blobs only when the tile's `led_scene` is active.
/// Glow is approximated via 4 stacked alpha-decreasing circles per LED —
/// a cheap gaussian stand-in that reads as soft light without the FBO machinery
/// the prior FemtoVG-based strip needed.
pub(super) fn paint_led_strip(
    painter: &egui::Painter,
    tile: &PreviewTile,
    tile_origin: egui::Pos2,
    time_s: f32,
) {
    let Some(led_count) = tile.led_count else {
        return;
    };
    let led_count = led_count as usize;
    if led_count == 0 {
        return;
    }

    let strip_w = tile.gpu.width as f32;
    let strip_h = LED_STRIP_H as f32;
    let strip_rect = egui::Rect::from_min_size(
        tile_origin + egui::vec2(tile.x as f32, tile.y as f32 + tile.gpu.height as f32),
        egui::vec2(strip_w, strip_h),
    );
    // Semi-transparent so the testbed checkerboard reads through the
    // diffuser gap — full black flattened the gap into a hard bar.
    painter.rect_filled(strip_rect, 0.0, egui::Color32::from_black_alpha(75));

    let Some(scene) = tile.led_scene.as_ref().filter(|_| tile.led_enabled) else {
        return;
    };
    let (cr, cg, cb) = match &scene.effect {
        bmc_led::data::LedEffect::None => return,
        bmc_led::data::LedEffect::Solid(c)
        | bmc_led::data::LedEffect::Breathe(c)
        | bmc_led::data::LedEffect::Chase(c)
        | bmc_led::data::LedEffect::KnightRider(c)
        | bmc_led::data::LedEffect::Scan(c)
        | bmc_led::data::LedEffect::Snake(c) => (c.r, c.g, c.b),
    };
    let period_s = scene.period.map_or(1.0, |d| d.as_secs_f32().max(0.1));
    let anim_t = time_s / period_s;

    // FULL tile: LEDs span centre half. Smaller tiles: full width.
    let is_full = tile.gpu.width >= 1280;
    let led_region_w = if is_full { strip_w * 0.5 } else { strip_w };
    let led_x_offset = (strip_w - led_region_w) / 2.0;
    let led_spacing = led_region_w / led_count as f32;

    for idx in 0..led_count {
        let phase = idx as f32 / led_count as f32;
        let brightness = led_brightness(scene.effect, phase, anim_t, led_count);
        if brightness <= 0.0 {
            continue;
        }
        let cx = strip_rect.min.x + led_x_offset + (idx as f32 + 0.5) * led_spacing;
        let cy = strip_rect.min.y;
        // Stacked falloff: 4 circles of increasing radius and decreasing alpha approximate
        // a radial gradient cheaply enough for the testbed UI.
        for ring in 0..4 {
            let t = ring as f32 / 3.0;
            let radius = led_spacing * (0.4 + t * 1.6);
            let alpha = (brightness * (1.0 - t) * (1.0 - t) * 0.8 * 255.0).clamp(0.0, 255.0) as u8;
            if alpha == 0 {
                continue;
            }
            let color = egui::Color32::from_rgba_unmultiplied(cr, cg, cb, alpha);
            painter.circle_filled(egui::pos2(cx, cy), radius, color);
        }
    }
}

// ── Timing chart ────────────────────────────────────────────────────

/// Paint a stacked bar chart of the most recent frame timings into `rect`.
/// One column per sample, stacked top-to-bottom: wasm / deserialize / layout / render / flush.
/// A horizontal reference line marks the 16.6 ms / 60 fps budget.
pub(super) fn paint_timing_chart(
    painter: &egui::Painter,
    rect: egui::Rect,
    samples: &[bmc_render::FrameTimings],
) {
    // Fixed column width — bars stay the same size and newest samples append at the right edge,
    // older samples scroll off the left. Avoids the "bars resize as the window fills" effect.
    const COL_W: f32 = 2.0;
    let max_cols = (rect.width() / COL_W).floor().max(1.0) as usize;
    let n = samples.len().min(max_cols);
    if n == 0 {
        return;
    }
    let start = samples.len() - n;
    let view = &samples[start..];

    // Peak total across the window establishes the y scale.
    let peak_us = view
        .iter()
        .map(|s| {
            u64::from(s.wasm_us)
                + u64::from(s.deserialize_us)
                + u64::from(s.layout_us)
                + u64::from(s.render_us)
                + u64::from(s.flush_us)
        })
        .max()
        .unwrap_or(1)
        .max(1);
    // y-scale floor is 36,000 µs — slightly above the 30 fps budget (33,333 µs)
    // so the 30 fps reference line sits a bit below the top of the chart and its label has room
    // instead of being clipped at the edge. A genuine spike past 36 ms grows the scale.
    let y_scale_us = peak_us.max(36_000) as f32;
    let col_w = COL_W;

    // Subtle horizontal grid every 5 ms — drawn first so bars overlay on top.
    let grid_color = egui::Color32::from_rgba_unmultiplied(140, 140, 140, 30);
    let grid_step_us = 5_000.0_f32;
    let mut grid_us = grid_step_us;
    while grid_us < y_scale_us {
        let y = rect.max.y - (grid_us / y_scale_us) * rect.height();
        if y > rect.min.y && y < rect.max.y {
            painter.line_segment(
                [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
                egui::Stroke::new(1.0_f32, grid_color),
            );
        }
        grid_us += grid_step_us;
    }
    // Component colours mirror PerfOverlay's legend ordering.
    let colours = [
        (egui::Color32::from_rgb(0x6A, 0x9F, 0xD8), "wasm"),
        (egui::Color32::from_rgb(0xE0, 0x9A, 0x50), "deser"),
        (egui::Color32::from_rgb(0xCC, 0xCC, 0x50), "layout"),
        (egui::Color32::from_rgb(0x50, 0xCC, 0x50), "render"),
        (egui::Color32::from_rgb(0xCC, 0x50, 0xCC), "flush"),
    ];

    // Right-anchored: oldest sample drawn at the leftmost slot used, newest at the right edge.
    let bars_left = rect.max.x - n as f32 * col_w;
    for (i, sample) in view.iter().enumerate() {
        let x = bars_left + i as f32 * col_w;
        let mut y = rect.max.y;
        let parts = [
            sample.wasm_us,
            sample.deserialize_us,
            sample.layout_us,
            sample.render_us,
            sample.flush_us,
        ];
        for (part_us, (color, _)) in parts.into_iter().zip(colours) {
            let h = (part_us as f32 / y_scale_us) * rect.height();
            if h < 0.5 {
                continue;
            }
            let bar =
                egui::Rect::from_min_max(egui::pos2(x, y - h), egui::pos2(x + col_w.max(1.0), y));
            painter.rect_filled(bar, 0.0, color);
            y -= h;
        }
    }

    // Reference lines at 60 fps (16.6 ms) and 30 fps (33.3 ms)
    // — the two budgets the testbed cares about. Each labeled at the right edge.
    for (us, label) in [(16_666.0_f32, "60 fps"), (33_333.0_f32, "30 fps")] {
        let y = rect.max.y - (us / y_scale_us) * rect.height();
        if y < rect.min.y || y > rect.max.y {
            continue;
        }
        painter.line_segment(
            [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
            egui::Stroke::new(
                1.0_f32,
                egui::Color32::from_rgba_unmultiplied(180, 180, 180, 140),
            ),
        );
        // Label sits BELOW the line so it doesn't get clipped against `rect.min.y`
        // when the line itself is near the top of the chart (e.g. 30 fps marker at peak scale).
        painter.text(
            egui::pos2(rect.max.x - 2.0, y + 1.0),
            egui::Align2::RIGHT_TOP,
            label,
            egui::FontId::monospace(9.0),
            egui::Color32::from_gray(200),
        );
    }
}

/// Paint the chart's component legend in its own strip — the colour swatches and labels
/// for wasm / deser / layout / render / flush, in stack order.
/// Lives above the chart so it doesn't overlap the bars.
pub(super) fn paint_timing_legend(painter: &egui::Painter, rect: egui::Rect) {
    let colours = [
        (egui::Color32::from_rgb(0x6A, 0x9F, 0xD8), "wasm"),
        (egui::Color32::from_rgb(0xE0, 0x9A, 0x50), "deser"),
        (egui::Color32::from_rgb(0xCC, 0xCC, 0x50), "layout"),
        (egui::Color32::from_rgb(0x50, 0xCC, 0x50), "render"),
        (egui::Color32::from_rgb(0xCC, 0x50, 0xCC), "flush"),
    ];
    let mut x_cursor = rect.min.x;
    let cy = rect.center().y;
    for (color, label) in colours {
        let sw = egui::Rect::from_center_size(egui::pos2(x_cursor + 4.0, cy), egui::vec2(8.0, 8.0));
        painter.rect_filled(sw, 0.0, color);
        painter.text(
            egui::pos2(x_cursor + 10.0, cy),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::monospace(9.0),
            egui::Color32::from_gray(180),
        );
        x_cursor += 56.0;
    }
}

// ── Round visible-area overlay ─────────────────────────────────────

/// For a column at horizontal offset `x` (0..=width) over a circle inscribed
/// in a `width` x `height` tile, return the heights of the outside-circle caps
/// at the top and bottom of that column, in pixels.
pub(super) fn circle_outside_spans(x: f32, width: f32, height: f32) -> (f32, f32) {
    let r = width.min(height) / 2.0;
    let cx = width / 2.0;
    let cy = height / 2.0;
    let dx = (x - cx).abs();
    if dx >= r {
        return (cy, height - cy);
    }
    let half_chord = (r * r - dx * dx).sqrt();
    let top = (cy - half_chord).max(0.0);
    let bottom = (height - (cy + half_chord)).max(0.0);
    (top, bottom)
}

/// Draw the round visible-area treatment over a tile rect.
pub(super) fn paint_round_overlay(painter: &egui::Painter, rect: egui::Rect) {
    if rect.width() < 2.0 || rect.height() < 2.0 {
        return;
    }
    let dim = egui::Color32::from_rgba_unmultiplied(10, 10, 14, 200);
    let cols = rect.width().ceil() as usize;
    for col in 0..cols {
        let x = col as f32;
        let (top, bottom) = circle_outside_spans(x, rect.width(), rect.height());
        let cx0 = rect.min.x + x;
        let cx1 = (cx0 + 1.0).min(rect.max.x);
        if top > 0.5 {
            let cap = egui::Rect::from_min_max(
                egui::pos2(cx0, rect.min.y),
                egui::pos2(cx1, rect.min.y + top),
            );
            painter.rect_filled(cap, 0.0, dim);
        }
        if bottom > 0.5 {
            let cap = egui::Rect::from_min_max(
                egui::pos2(cx0, rect.max.y - bottom),
                egui::pos2(cx1, rect.max.y),
            );
            painter.rect_filled(cap, 0.0, dim);
        }
    }
    let center = rect.center();
    let radius = rect.width().min(rect.height()) / 2.0;
    painter.circle_stroke(
        center,
        radius,
        egui::Stroke::new(
            1.5_f32,
            egui::Color32::from_rgba_unmultiplied(200, 200, 220, 160),
        ),
    );
}

pub(super) fn is_round(shape: DisplayShape) -> bool {
    matches!(shape, DisplayShape::Round)
}

// ── Perf report ─────────────────────────────────────────────────────

pub(super) fn write_perf_report(
    path: &Path,
    samples: &[bmc_render::FrameTimings],
    section_samples: &[std::collections::BTreeMap<String, u64>],
) {
    let n = samples.len();
    if n == 0 {
        return;
    }
    let sum_wasm: u64 = samples.iter().map(|s| u64::from(s.wasm_us)).sum();
    let sum_deser: u64 = samples.iter().map(|s| u64::from(s.deserialize_us)).sum();
    let sum_layout: u64 = samples.iter().map(|s| u64::from(s.layout_us)).sum();
    let sum_render: u64 = samples.iter().map(|s| u64::from(s.render_us)).sum();
    let sum_flush: u64 = samples.iter().map(|s| u64::from(s.flush_us)).sum();
    let n_u64 = n as u64;
    let avg = |s: u64| s / n_u64;

    // Average over frames the section fired in, not all frames — cached-tree
    // frames run no guest code and would skew the per-frame cost down.
    let mut totals: std::collections::BTreeMap<&str, (u64, u64)> =
        std::collections::BTreeMap::new();
    for frame in section_samples {
        for (name, &fuel) in frame {
            let entry = totals.entry(name.as_str()).or_default();
            entry.0 += fuel;
            entry.1 += 1;
        }
    }
    let fuel_per_frame: serde_json::Map<String, serde_json::Value> = totals
        .into_iter()
        .map(|(name, (sum, frames))| ((*name).to_owned(), serde_json::json!(sum / frames)))
        .collect();

    // Per-frame series for `perf_finalize.py` to build Firefox-format counters.
    // Each fuel series is zero on cached-tree frames (no guest code ran).
    let section_names: std::collections::BTreeSet<&str> = section_samples
        .iter()
        .flat_map(|f| f.keys().map(String::as_str))
        .collect();
    let fuel_series: serde_json::Map<String, serde_json::Value> = section_names
        .into_iter()
        .map(|name| {
            let series: Vec<u64> = section_samples
                .iter()
                .map(|f| f.get(name).copied().unwrap_or(0))
                .collect();
            (name.to_owned(), serde_json::json!(series))
        })
        .collect();
    let per_frame = serde_json::json!({
        "wasm_us": samples.iter().map(|s| s.wasm_us).collect::<Vec<_>>(),
        "deserialize_us": samples.iter().map(|s| s.deserialize_us).collect::<Vec<_>>(),
        "layout_us": samples.iter().map(|s| s.layout_us).collect::<Vec<_>>(),
        "render_us": samples.iter().map(|s| s.render_us).collect::<Vec<_>>(),
        "flush_us": samples.iter().map(|s| s.flush_us).collect::<Vec<_>>(),
        "fuel": fuel_series,
    });

    let mut report = serde_json::json!({
        "frames": n,
        "avg_us": {
            "wasm": avg(sum_wasm),
            "deserialize": avg(sum_deser),
            "layout": avg(sum_layout),
            "render": avg(sum_render),
            "flush": avg(sum_flush),
        },
        "per_frame": per_frame,
    });
    if !fuel_per_frame.is_empty() {
        report["fuel_per_frame"] = serde_json::Value::Object(fuel_per_frame);
    }
    match std::fs::write(
        path,
        serde_json::to_string_pretty(&report).unwrap_or_default(),
    ) {
        Ok(()) => println!("Perf report written to {}", path.display()),
        Err(e) => tracing::warn!("perf report: {e}"),
    }
}

#[cfg(test)]
mod round_tests {
    use super::circle_outside_spans;

    #[test]
    fn center_column_has_no_outside_span() {
        let (top, bottom) = circle_outside_spans(240.0, 480.0, 480.0);
        assert!(top.abs() < 0.5, "top dim height near zero at centre: {top}");
        assert!(
            bottom.abs() < 0.5,
            "bottom dim height near zero at centre: {bottom}"
        );
    }

    #[test]
    fn edge_column_is_fully_outside() {
        let (top, bottom) = circle_outside_spans(0.0, 480.0, 480.0);
        assert!(
            (top + bottom) >= 479.0,
            "edge column fully dimmed: {top}+{bottom}"
        );
    }

    #[test]
    fn quarter_column_dims_symmetric_caps() {
        let (top, bottom) = circle_outside_spans(120.0, 480.0, 480.0);
        assert!(
            (top - bottom).abs() < 0.5,
            "caps symmetric: {top} vs {bottom}"
        );
        assert!(top > 0.0, "some dimming expected off-centre: {top}");
    }
}
