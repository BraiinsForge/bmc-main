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
use egui_glow::glow::HasContext as _;

use bmc_wasm_runtime::platform_catalog::DisplayShape;

use super::view::DeviceView;

// ── GL helpers ──────────────────────────────────────────────────────

/// What a view draws into, in the context that created it.
///
/// A framebuffer is a container object, which a share group does *not* share,
/// so it can only be created and destroyed by the context that renders through
/// it. The colour texture is shared, and its name is all the compositor needs.
/// The painter's registration is therefore held apart from these, by the
/// thread that owns the painter rather than the one that draws.
pub(super) struct ViewTargets {
    fbo: egui_glow::glow::Framebuffer,
    rbo: egui_glow::glow::Renderbuffer,
    texture: egui_glow::glow::Texture,
    pub(super) width: u32,
    pub(super) height: u32,
}

impl ViewTargets {
    /// Allocate a `width × height` RGBA8 colour texture and a framebuffer that
    /// draws into it, on whichever context is current.
    pub(super) fn create(gl: &egui_glow::glow::Context, width: u32, height: u32) -> Result<Self> {
        // SAFETY: the caller holds a context current on this thread.
        unsafe {
            let texture = gl
                .create_texture()
                .map_err(|e| anyhow::anyhow!("create_texture: {e}"))?;
            configure_texture(gl, texture, width, height);
            let (fbo, rbo) = create_render_target(gl, texture, width, height)?;
            Ok(Self {
                fbo,
                rbo,
                texture,
                width,
                height,
            })
        }
    }

    /// Numeric FBO ID for `WasmWidgetRuntime::new(... fbo_id ...)`.
    pub(super) fn fbo_id(&self) -> u32 {
        self.fbo.0.get()
    }

    /// GL name of the colour texture — the one part of these targets that a
    /// share group shares, and so the only part another context can name.
    pub(super) fn texture_name(&self) -> u32 {
        self.texture.0.get()
    }

    /// Hand the colour texture to the painter, which owns it from then on —
    /// including deleting it, which is why `destroy_container_objects` leaves
    /// it alone. Only the thread the painter runs on may call this.
    pub(super) fn register(&self, painter: &mut egui_glow::Painter) -> egui::TextureId {
        painter.register_native_texture(self.texture)
    }

    /// Delete the objects a share group does not share, in the context that
    /// created them. The colour texture is not among them: the painter deletes
    /// it with the registration, and deleting it here would double free.
    pub(super) fn destroy_container_objects(self, gl: &egui_glow::glow::Context) {
        // SAFETY: the caller holds the creating context current on this thread.
        unsafe {
            gl.delete_framebuffer(self.fbo);
            gl.delete_renderbuffer(self.rbo);
        }
    }

    /// Delete everything, texture included — for targets that were never
    /// registered with the painter, whose texture nobody else will free.
    pub(super) fn destroy(self, gl: &egui_glow::glow::Context) {
        let texture = self.texture;
        self.destroy_container_objects(gl);
        // SAFETY: the caller holds the creating context current on this thread.
        unsafe {
            gl.delete_texture(texture);
        }
    }

    /// Copy this target's colour into `dest`, on whichever context is current.
    ///
    /// This is the frame handoff for a threaded view: the renderer always
    /// draws into one target — femtovg believes it is the screen, and its
    /// drop-shadow pass restores `RenderTarget::Screen` mid-frame on that
    /// assumption (`bmc-render/src/gpu/renderer.rs`, `drop_shadow`) — so the
    /// double buffering happens by copying frames out, never by retargeting.
    pub(super) fn blit_to(&self, gl: &egui_glow::glow::Context, dest: &ViewTargets) {
        // SAFETY: the caller holds the creating context current on this
        // thread, and both targets were created against it.
        unsafe {
            gl.bind_framebuffer(egui_glow::glow::READ_FRAMEBUFFER, Some(self.fbo));
            gl.bind_framebuffer(egui_glow::glow::DRAW_FRAMEBUFFER, Some(dest.fbo));
            gl.blit_framebuffer(
                0,
                0,
                self.width as i32,
                self.height as i32,
                0,
                0,
                dest.width as i32,
                dest.height as i32,
                egui_glow::glow::COLOR_BUFFER_BIT,
                egui_glow::glow::NEAREST,
            );
            gl.bind_framebuffer(egui_glow::glow::FRAMEBUFFER, None);
        }
    }
}

fn configure_texture(
    gl: &egui_glow::glow::Context,
    texture: egui_glow::glow::Texture,
    width: u32,
    height: u32,
) {
    // SAFETY: the context is current on this thread for the window's lifetime.
    unsafe {
        gl.bind_texture(egui_glow::glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            egui_glow::glow::TEXTURE_2D,
            0,
            egui_glow::glow::RGBA8 as i32,
            width as i32,
            height as i32,
            0,
            egui_glow::glow::RGBA,
            egui_glow::glow::UNSIGNED_BYTE,
            egui_glow::glow::PixelUnpackData::Slice(None),
        );
        gl.tex_parameter_i32(
            egui_glow::glow::TEXTURE_2D,
            egui_glow::glow::TEXTURE_MIN_FILTER,
            egui_glow::glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            egui_glow::glow::TEXTURE_2D,
            egui_glow::glow::TEXTURE_MAG_FILTER,
            egui_glow::glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            egui_glow::glow::TEXTURE_2D,
            egui_glow::glow::TEXTURE_WRAP_S,
            egui_glow::glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            egui_glow::glow::TEXTURE_2D,
            egui_glow::glow::TEXTURE_WRAP_T,
            egui_glow::glow::CLAMP_TO_EDGE as i32,
        );
        gl.bind_texture(egui_glow::glow::TEXTURE_2D, None);
    }
}

fn create_render_target(
    gl: &egui_glow::glow::Context,
    texture: egui_glow::glow::Texture,
    width: u32,
    height: u32,
) -> Result<(egui_glow::glow::Framebuffer, egui_glow::glow::Renderbuffer)> {
    // SAFETY: the context is current on this thread for the window's lifetime.
    unsafe {
        let fbo = gl
            .create_framebuffer()
            .map_err(|e| anyhow::anyhow!("create_framebuffer: {e}"))?;
        gl.bind_framebuffer(egui_glow::glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            egui_glow::glow::FRAMEBUFFER,
            egui_glow::glow::COLOR_ATTACHMENT0,
            egui_glow::glow::TEXTURE_2D,
            Some(texture),
            0,
        );

        // Stencil renderbuffer — FemtoVG's stroke shader uses stencil.
        let rbo = gl
            .create_renderbuffer()
            .map_err(|e| anyhow::anyhow!("create_renderbuffer: {e}"))?;
        gl.bind_renderbuffer(egui_glow::glow::RENDERBUFFER, Some(rbo));
        gl.renderbuffer_storage(
            egui_glow::glow::RENDERBUFFER,
            egui_glow::glow::DEPTH24_STENCIL8,
            width as i32,
            height as i32,
        );
        gl.framebuffer_renderbuffer(
            egui_glow::glow::FRAMEBUFFER,
            egui_glow::glow::DEPTH_STENCIL_ATTACHMENT,
            egui_glow::glow::RENDERBUFFER,
            Some(rbo),
        );
        gl.bind_renderbuffer(egui_glow::glow::RENDERBUFFER, None);

        let status = gl.check_framebuffer_status(egui_glow::glow::FRAMEBUFFER);
        if status != egui_glow::glow::FRAMEBUFFER_COMPLETE {
            anyhow::bail!("FBO incomplete: {status:#x}");
        }
        gl.bind_framebuffer(egui_glow::glow::FRAMEBUFFER, None);

        Ok((fbo, rbo))
    }
}

/// Wraps the GL loader into the `&str`-keyed shape `WasmWidgetRuntime::new`
/// accepts. glutin keys on `&CStr`, and the `CString` is allocated per call
/// since the runtime constructor only runs once per widget construction.
pub(super) fn proc_loader(get_proc: GlProcAddress) -> impl FnMut(&str) -> *const std::ffi::c_void {
    move |name: &str| {
        let Ok(cstr) = CString::new(name) else {
            return std::ptr::null();
        };
        get_proc(&cstr)
    }
}

/// GL function loader closure shape — `&CStr` → raw function pointer.
/// Aliased so the `dyn Fn` trait object isn't spelled out at every storage site.
pub(super) type GlProcAddress =
    Arc<dyn Fn(&std::ffi::CStr) -> *const std::ffi::c_void + Send + Sync>;

// ── Background ──────────────────────────────────────────────────────

/// Paint a two-tone checkerboard over `rect` — same pattern as `bmc-virt-console`'s
/// device backdrop so the tile boundaries read clearly against an otherwise blank window.
pub(super) fn draw_checkerboard(
    painter: &egui::Painter,
    rect: egui::Rect,
    palette: &super::theme::Palette,
) {
    let size = 16.0;
    let color_a = palette.canvas;
    let color_b = palette.canvas_alt;
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

// ── LED strip rendering ─────────────────────────────────────────────

/// Collects what the driver would clock out to the strip, so the preview runs
/// the device's own effects rather than a second implementation of them.
#[derive(Default)]
struct StripSink {
    pixels: Vec<bmc_led::apa102_spi::Apa102Pixel>,
}

impl bmc_led::apa102_spi::SmartLedsWrite for StripSink {
    type Error = std::convert::Infallible;
    type Color = bmc_led::apa102_spi::Apa102Pixel;

    fn write<T, I>(&mut self, iterator: T) -> Result<(), Self::Error>
    where
        T: IntoIterator<Item = I>,
        I: Into<Self::Color>,
    {
        self.pixels.clear();
        self.pixels.extend(iterator.into_iter().map(Into::into));
        Ok(())
    }
}

/// Where in its cycle `scene` sits at `time_s`.
/// A scene with no period is static, which the driver reports as zero.
fn scene_phase(scene: &bmc_led::data::LedScene, time_s: f32) -> f32 {
    scene.period.map_or(0.0, |period| {
        (time_s / period.as_secs_f32().max(0.1)).fract()
    })
}

/// Run `scene` through the device's effect code and return each LED's light
/// as linear RGB in 0..1, already carrying the strip's global brightness.
fn device_pixels(scene: &bmc_led::data::LedScene, phase: f32) -> Vec<[f32; 3]> {
    use bmc_led::apa102_spi::{config, effects};

    let mut sink = StripSink::default();
    // The driver's own default; the testbed exposes no brightness control.
    let brightness = config::APA102_MAX_BRIGHTNESS;
    match scene.effect {
        bmc_led::data::LedEffect::Snake(color) => {
            effects::update_snake(phase, config::SNAKE_LEN, brightness, color, &mut sink);
        }
        bmc_led::data::LedEffect::Chase(color) => {
            effects::update_chase(phase, config::SNAKE_LEN, brightness, color, &mut sink);
        }
        bmc_led::data::LedEffect::Scan(color) => {
            effects::update_scan(phase, config::SNAKE_LEN, brightness, color, &mut sink);
        }
        bmc_led::data::LedEffect::KnightRider(color) => {
            effects::update_knight_rider(
                phase,
                config::SNAKE_LEN,
                config::SNAKE_LEN + 1,
                brightness,
                color,
                &mut sink,
            );
        }
        bmc_led::data::LedEffect::Breathe(color) => {
            effects::update_breathe(phase, brightness, color, &mut sink);
        }
        bmc_led::data::LedEffect::Solid(color) => {
            effects::update_solid(brightness, color, &mut sink);
        }
        bmc_led::data::LedEffect::None => effects::update_none(&mut sink),
    }

    sink.pixels
        .iter()
        .map(|pixel| {
            // APA102 drives RGB against a separate global current setting,
            // so what the eye sees is their product.
            let gain =
                f32::from(u8::from(pixel.brightness)) / f32::from(config::APA102_MAX_BRIGHTNESS);
            [
                f32::from(pixel.red) / 255.0 * gain,
                f32::from(pixel.green) / 255.0 * gain,
                f32::from(pixel.blue) / 255.0 * gain,
            ]
        })
        .collect()
}

/// Mesh columns per LED. Vertex interpolation covers the gaps, so this only
/// has to be fine enough that the blurred curve stops faceting.
const DIFFUSER_COLS_PER_LED: usize = 3;

/// How many neighbours a lit LED bleeds into, the diffuser being strong enough
/// that adjacent LEDs read as one wash.
const DIFFUSER_SPREAD: isize = 2;

/// Depth down the strip paired with the light left at that depth.
/// Three rows, not two: a straight fade reads as a painted gradient.
const DIFFUSER_FALLOFF: [(f32, f32); 3] = [(0.0, 0.85), (0.45, 0.3), (1.0, 0.0)];

/// Spread each LED into its neighbours, so what reaches the eye is
/// the diffuser's blend rather than the individual sources behind it.
fn diffuse_levels(raw: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let last = raw.len().saturating_sub(1) as isize;
    raw.iter()
        .enumerate()
        .map(|(idx, _)| {
            let (mut sum, mut total) = ([0.0; 3], 0.0);
            for offset in -DIFFUSER_SPREAD..=DIFFUSER_SPREAD {
                // Falls off with distance, so a lit LED still dominates its own column.
                let weight = 1.0 / (1.0 + (offset * offset) as f32);
                // Clamped rather than treated as unlit, so a solid scene stays
                // even instead of dimming towards both ends.
                let neighbour = (idx as isize + offset).clamp(0, last) as usize;
                for channel in 0..3 {
                    sum[channel] += raw[neighbour][channel] * weight;
                }
                total += weight;
            }
            sum.map(|channel| channel / total)
        })
        .collect()
}

/// Light at `u` across the LED region, where 0 and 1 are its outer edges.
/// Falls to zero past the outermost LED: on the fullscreen tile they cover
/// only the centre half, and holding the end value would wash the whole strip.
fn sample_level(levels: &[[f32; 3]], u: f32) -> [f32; 3] {
    // LED centres sit half a spacing in from each edge.
    let pos = u * levels.len() as f32 - 0.5;
    let lower = pos.floor();
    let frac = (pos - lower).clamp(0.0, 1.0);
    let at = |idx: isize| -> [f32; 3] {
        usize::try_from(idx)
            .ok()
            .and_then(|idx| levels.get(idx))
            .copied()
            .unwrap_or([0.0; 3])
    };
    let (lo, hi) = (at(lower as isize), at(lower as isize + 1));
    std::array::from_fn(|channel| lo[channel] * (1.0 - frac) + hi[channel] * frac)
}

/// Split emitted light into a colour and how strongly it reads.
/// The effects dim towards black; carrying that as alpha instead keeps an
/// unlit LED transparent rather than painting the strip darker.
fn light_to_paint(light: [f32; 3]) -> (egui::Color32, f32) {
    let peak = light[0].max(light[1]).max(light[2]);
    if peak <= 0.0 {
        return (egui::Color32::TRANSPARENT, 0.0);
    }
    let channel = |value: f32| (value / peak * 255.0).clamp(0.0, 255.0) as u8;
    (
        egui::Color32::from_rgb(channel(light[0]), channel(light[1]), channel(light[2])),
        peak,
    )
}

/// Fraction of the device width the LED region spans, centred.
/// BMC100's ten LEDs sit under the middle of the enclosure, not edge to edge.
const LED_REGION_FRACTION: f32 = 0.5;

/// Paint a device frame's LED glow into `strip_rect`; the caller draws the
/// diffuser plate underneath, since the plate is enclosure and this is light.
///
/// The strip reads through a strong diffuser, mostly bounced off the surface
/// below, so it paints as one blended wash, not ten distinct sources.
pub(super) fn paint_led_strip(
    painter: &egui::Painter,
    scene_view: Option<&DeviceView>,
    strip_rect: egui::Rect,
    time_s: f32,
) {
    let strip_w = strip_rect.width();
    let strip_h = strip_rect.height();

    let Some(scene) = scene_view.and_then(DeviceView::led_scene) else {
        return;
    };
    if matches!(scene.effect, bmc_led::data::LedEffect::None) {
        return;
    }
    let levels = diffuse_levels(&device_pixels(scene, scene_phase(scene, time_s)));

    let led_region_w = strip_w * LED_REGION_FRACTION;
    let led_x_offset = (strip_w - led_region_w) / 2.0;

    // Light washes past its own LEDs, so the mesh spans the whole strip.
    let mut mesh = egui::Mesh::default();
    let cols = levels.len() * DIFFUSER_COLS_PER_LED;
    for col in 0..=cols {
        let x = strip_rect.min.x + strip_w * (col as f32 / cols as f32);
        let light = sample_level(
            &levels,
            (x - strip_rect.min.x - led_x_offset) / led_region_w,
        );
        let (color, level) = light_to_paint(light);
        for (depth, falloff) in DIFFUSER_FALLOFF {
            let alpha = (level * falloff * 255.0).clamp(0.0, 255.0) as u8;
            mesh.colored_vertex(
                egui::pos2(x, strip_rect.min.y + strip_h * depth),
                egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha),
            );
        }
    }
    let rows = DIFFUSER_FALLOFF.len() as u32;
    for col in 0..cols as u32 {
        for row in 0..rows - 1 {
            let near = col * rows + row;
            let far = (col + 1) * rows + row;
            mesh.add_triangle(near, near + 1, far);
            mesh.add_triangle(near + 1, far + 1, far);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

// ── Timing chart ────────────────────────────────────────────────────

/// Paint a stacked bar chart of the most recent frame timings into `rect`.
/// One column per sample, stacked top-to-bottom: wasm / deserialize / layout / render / flush.
/// A horizontal reference line marks the 16.6 ms / 60 fps budget.
/// Fixed column width — bars stay the same size and newest samples append at
/// the right edge, older samples scroll off the left. Avoids the "bars resize
/// as the window fills" effect.
pub(super) const CHART_COL_W: f32 = 2.0;

/// The frame's components, in the order the bars stack them. Their colours are
/// `palette.data` at the same index, so the chart and its legend cannot drift.
const COMPONENT_LABELS: [&str; 5] = ["wasm", "deser", "layout", "render", "flush"];

pub(super) fn paint_timing_chart(
    painter: &egui::Painter,
    rect: egui::Rect,
    samples: &[bmc_render::FrameTimings],
    palette: &super::theme::Palette,
) {
    // Sunk into the bar it sits in, like the sidebar's grouped controls.
    painter.rect_filled(rect, 0.0, palette.layer_inset);
    let max_cols = (rect.width() / CHART_COL_W).floor().max(1.0) as usize;
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
    // The 60 fps frame budget as a floor, so a bar's height means cost and not
    // rank: scaled to the window's own peak, a steady frame fills the chart and
    // the first render flattens every frame after it. A spike past the budget
    // still grows the scale.
    let y_scale_us = peak_us.max(16_667) as f32;
    let col_w = CHART_COL_W;

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
        for (part_us, color) in parts.into_iter().zip(palette.data) {
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

    // The two budgets worth marking. 60 fps is the scale's own floor, so it
    // rides the top edge until a spike grows past it and pushes it down into
    // the chart; 30 fps appears only once one has. Unlabelled: at this height
    // a 9 pt label leaves nothing to read but itself.
    for us in [16_666.0_f32, 33_333.0_f32] {
        let y = rect.max.y - (us / y_scale_us) * rect.height();
        if y < rect.min.y || y > rect.max.y {
            continue;
        }
        painter.line_segment(
            [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
            egui::Stroke::new(1.0_f32, palette.text_disabled.gamma_multiply(0.4)),
        );
    }
}

/// What one view spent on its last frame, over the corner of that view.
///
/// A corner overlay rather than a strip: a frame can hold several views
/// inside one faithful device mock, and anything that adds height would
/// displace the mock's own geometry.
///
/// Deliberately unthemed. It reads over whatever the widget drew, which is
/// neither of the testbed's two backgrounds, so it carries its own contrast.
pub(super) fn paint_view_timings(
    painter: &egui::Painter,
    view: egui::Rect,
    timings: &bmc_render::FrameTimings,
    slip_ms: Option<u64>,
) {
    let line_h: f32 = 11.0;
    let rows = [
        ("wasm", format!("{:>5} µs", timings.wasm_us)),
        ("lay", format!("{:>5} µs", timings.layout_us)),
        ("ren", format!("{:>5} µs", timings.render_us)),
        (
            "slip",
            slip_ms.map_or_else(|| "    — ms".to_owned(), |ms| format!("{ms:>5} ms")),
        ),
    ];
    #[expect(
        clippy::cast_precision_loss,
        reason = "four rows of a fixed-size overlay"
    )]
    let size = egui::vec2(96.0, line_h.mul_add(rows.len() as f32, 4.0));
    // Skip a view the overlay would cover more than a quarter of:
    // an instrument that hides what it measures reads as a broken render.
    if view.width() < size.x * 2.0 || view.height() < size.y * 2.0 {
        return;
    }

    let rect = egui::Rect::from_min_size(view.min, size);
    painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(190));
    for (idx, (label, value)) in rows.iter().enumerate() {
        #[expect(
            clippy::cast_precision_loss,
            reason = "four rows of a fixed-size overlay"
        )]
        let y = line_h.mul_add(idx as f32, rect.min.y + 2.0);
        painter.text(
            egui::pos2(rect.min.x + 4.0, y),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::monospace(9.0),
            egui::Color32::from_gray(150),
        );
        painter.text(
            egui::pos2(rect.max.x - 4.0, y),
            egui::Align2::RIGHT_TOP,
            value,
            egui::FontId::monospace(9.0),
            egui::Color32::from_gray(230),
        );
    }
}

/// Paint the chart's component legend in its own strip — the colour swatches and labels
/// for wasm / deser / layout / render / flush, in stack order.
/// Lives above the chart so it doesn't overlap the bars.
pub(super) fn paint_timing_legend(
    painter: &egui::Painter,
    rect: egui::Rect,
    text: egui::Color32,
    palette: &super::theme::Palette,
) {
    /// Side of a series swatch, and the gap between it and the label it names.
    const SWATCH: f32 = 8.0;
    const SWATCH_GAP: f32 = 4.0;
    /// One entry to the next, wide enough for the longest label.
    const ENTRY_STRIDE: f32 = 56.0;

    let cy = rect.center().y;
    for (i, (label, color)) in COMPONENT_LABELS.into_iter().zip(palette.data).enumerate() {
        let left = rect.min.x + i as f32 * ENTRY_STRIDE;
        let swatch = egui::Rect::from_center_size(
            egui::pos2(left + SWATCH / 2.0, cy),
            egui::Vec2::splat(SWATCH),
        );
        painter.rect_filled(swatch, 0.0, color);
        painter.text(
            egui::pos2(left + SWATCH + SWATCH_GAP, cy),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::monospace(9.0),
            text,
        );
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
///
/// The corners are scrimmed rather than filled, so a widget's overdraw
/// outside the circle stays visible to whoever caused it.
pub(super) fn paint_round_overlay(
    painter: &egui::Painter,
    rect: egui::Rect,
    palette: &super::theme::Palette,
) {
    if rect.width() < 2.0 || rect.height() < 2.0 {
        return;
    }
    let dim = palette.display_unlit;
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
        egui::Stroke::new(1.5_f32, palette.border_subtle),
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
mod effect_tests {
    use bmc_led::data::{LedEffect, LedScene, Rgb};

    use super::{device_pixels, scene_phase};

    const CYAN: Rgb = Rgb {
        r: 0,
        g: 255,
        b: 255,
    };

    fn scene(effect: LedEffect) -> LedScene {
        LedScene {
            effect,
            period: Some(std::time::Duration::from_secs(1)),
            duration: None,
        }
    }

    fn lit(pixels: &[[f32; 3]]) -> Vec<usize> {
        pixels
            .iter()
            .enumerate()
            .filter(|(_, light)| light.iter().any(|channel| *channel > 0.0))
            .map(|(idx, _)| idx)
            .collect()
    }

    #[test]
    fn the_whole_strip_is_driven() {
        let pixels = device_pixels(&scene(LedEffect::Solid(CYAN)), 0.0);
        assert_eq!(
            pixels.len(),
            usize::from(bmc_led::config::LED_COUNT),
            "one value per LED on the strip"
        );
    }

    #[test]
    fn unlit_leds_emit_nothing() {
        let pixels = device_pixels(&scene(LedEffect::Snake(CYAN)), 0.0);
        assert!(
            lit(&pixels).len() < pixels.len(),
            "the snake covers part of the strip, so the rest must be dark"
        );
    }

    /// The device draws a bar of `SNAKE_LEN`, not a gradient down the strip.
    #[test]
    fn the_snake_is_a_short_contiguous_run() {
        let pixels = device_pixels(&scene(LedEffect::Snake(CYAN)), 0.0);
        let lit = lit(&pixels);
        assert!(
            lit.len() <= usize::from(bmc_led::apa102_spi::config::SNAKE_LEN),
            "at most the snake's own length is lit, got {lit:?}"
        );
        for pair in lit.windows(2) {
            assert_eq!(pair[1], pair[0] + 1, "the body is contiguous: {lit:?}");
        }
    }

    #[test]
    fn the_snake_advances_with_phase() {
        let start = lit(&device_pixels(&scene(LedEffect::Snake(CYAN)), 0.0));
        let later = lit(&device_pixels(&scene(LedEffect::Snake(CYAN)), 0.5));
        assert_ne!(
            start, later,
            "half a period moves the snake along the strip"
        );
    }

    #[test]
    fn a_scene_without_a_period_holds_still() {
        let held = LedScene {
            effect: LedEffect::Snake(CYAN),
            period: None,
            duration: None,
        };
        assert!(
            scene_phase(&held, 9.0) <= 0.0,
            "the driver reports no phase for a scene that does not animate"
        );
    }

    #[test]
    fn a_periodic_scene_cycles_once_per_period() {
        let cycling = scene(LedEffect::Snake(CYAN));
        assert!(
            scene_phase(&cycling, 0.25) > 0.0,
            "it advances within a period"
        );
        assert!(
            scene_phase(&cycling, 2.0) <= 0.0,
            "and wraps at the end of one, so two whole periods land back at the start"
        );
    }
}

#[cfg(test)]
mod diffuser_tests {
    use super::{diffuse_levels, sample_level};

    /// Green channel only; the diffuser treats each the same way.
    fn green(levels: &[[f32; 3]]) -> Vec<f32> {
        levels.iter().map(|light| light[1]).collect()
    }

    #[test]
    fn a_solid_scene_stays_even_across_the_strip() {
        for level in green(&diffuse_levels(&[[0.0, 1.0, 0.0]; 10])) {
            assert!(
                (level - 1.0).abs() < 1e-6,
                "every LED lit should diffuse to an even wash, got {level}"
            );
        }
    }

    #[test]
    fn a_single_lit_led_bleeds_into_its_neighbours() {
        let mut raw = [[0.0; 3]; 10];
        raw[5] = [0.0, 1.0, 0.0];
        let levels = green(&diffuse_levels(&raw));
        assert!(levels[5] > levels[4], "the lit LED stays the brightest");
        assert!(levels[4] > 0.0, "its neighbour picks up light");
        assert!(levels[3] > 0.0, "and so does the one beyond it");
        assert!(levels[2] <= 0.0, "but not past the diffuser's reach");
    }

    #[test]
    fn light_stops_past_the_outermost_led() {
        let levels = [[0.0, 1.0, 0.0]; 4];
        assert!(
            sample_level(&levels, 1.5)[1] <= 0.0,
            "nothing emits past the LED region, so the wash cannot continue there"
        );
        assert!(sample_level(&levels, -0.5)[1] <= 0.0);
    }

    #[test]
    fn an_led_centre_samples_its_own_level() {
        let levels = [[0.0, 0.25, 0.0], [0.0, 0.75, 0.0]];
        // Two LEDs put their centres a quarter and three quarters across.
        assert!((sample_level(&levels, 0.25)[1] - 0.25).abs() < 1e-6);
        assert!((sample_level(&levels, 0.75)[1] - 0.75).abs() < 1e-6);
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
