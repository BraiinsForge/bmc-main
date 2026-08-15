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

//! Progress bar / slider component.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use bmc_wasm_protocol::{Color, ProgressKind};

use crate::renderer::{RenderTarget, Renderer};
use crate::tree::{AnimationContext, SliderSkinData};

// ── Data ─────────────────────────────────────────────────────────────

/// Host-side progress bar rendering data.
#[derive(Clone, Default, Debug)]
pub(crate) struct ProgressBarData {
    pub track_h: f32,
    pub mode: ProgressKind,
    pub fraction: f32,
    pub active: bool,
    pub fill_color: Color,
    pub track_color: Color,
    pub bg_color: Color,
    pub skin: Option<SliderSkinData>,
}

// ── Constants ────────────────────────────────────────────────────────

const PB_WAVE_POINTS_PER_CYCLE: usize = 8;
const PB_WAVE_LENGTH: f32 = 16.0;

// ── Rendering ────────────────────────────────────────────────────────

/// Render a host-side progress bar. Returns `true` if animations are active
/// (caller should request next frame).
pub(crate) fn render_progress_bar(
    renderer: &mut RenderTarget<'_, '_, '_>,
    pb: &ProgressBarData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    anim_ctx: &mut AnimationContext<'_>,
) -> bool {
    if let Some(skin) = &pb.skin {
        render_progress_bar_skinned(renderer, pb, skin, x, y, w, h)
    } else {
        render_progress_bar_flat(&mut *renderer, pb, x, y, w, h, anim_ctx)
    }
}

/// Flat (unskinned) progress bar: rects, circles, squiggle.
fn render_progress_bar_flat(
    renderer: &mut dyn Renderer,
    pb: &ProgressBarData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    anim_ctx: &mut AnimationContext<'_>,
) -> bool {
    let track_h = pb.track_h;
    let thumb_radius = track_h * 2.0;
    let half_track = track_h / 2.0;
    // Center on the laid-out box, which flex may have grown or (for the
    // thumb-less meter) shrunk past the notional bar height.
    let mid_y = y + h / 2.0;
    let is_indeterminate = pb.mode == ProgressKind::Indeterminate;
    let fraction = pb.fraction.clamp(0.0, 1.0);
    let fill_w = w * fraction;

    // Meter: rounded track and fill, no drag thumb, no squiggle.
    if pb.mode == ProgressKind::Meter {
        let track_y = mid_y - half_track;
        renderer.fill_rounded_rect(x, track_y, w, track_h, half_track, pb.track_color);
        if fill_w > 0.0 {
            // A fill narrower than the pill's end caps would render inverted;
            // clamp so the smallest non-zero progress shows as a dot-sized pill.
            let fill_w = fill_w.max(track_h);
            renderer.fill_rounded_rect(x, track_y, fill_w, track_h, half_track, pb.fill_color);
        }
        return false;
    }

    let mut animating = false;

    if is_indeterminate && pb.active {
        // Full-width animated squiggle
        render_squiggle(renderer, x, mid_y, w, track_h, pb.fill_color, anim_ctx);
        animating = true;
    } else {
        // Background track (full width)
        renderer.fill_rect(x, mid_y - half_track, w, track_h, pb.track_color);

        if pb.active && fill_w > track_h {
            // Animated squiggle on the filled portion
            render_squiggle(renderer, x, mid_y, fill_w, track_h, pb.fill_color, anim_ctx);

            // Clip rect: hide squiggle past the drag thumb
            let clip_x = x + fill_w;
            renderer.fill_rect(clip_x, y, w - fill_w + 1.0, h, pb.bg_color);

            // Remaining track after the drag thumb
            let track_x = clip_x + thumb_radius;
            renderer.fill_rect(
                track_x,
                mid_y - half_track,
                (w - fill_w - thumb_radius).max(0.0),
                track_h,
                pb.track_color,
            );
            animating = true;
        } else if fill_w > 0.0 {
            // Static fill (not active, or fill too small for squiggle)
            renderer.fill_rect(x, mid_y - half_track, fill_w, track_h, pb.fill_color);
        }

        // Drag thumb — clamp center so it never clips outside the bar
        {
            let thumb_cx = (x + fill_w).clamp(x + thumb_radius, x + w - thumb_radius);
            renderer.fill_circle(thumb_cx, mid_y, thumb_radius, pb.fill_color);
        }
    }

    animating
}

/// Skinned progress bar: 9-patch track + bitmap thumb.
fn render_progress_bar_skinned(
    renderer: &mut RenderTarget<'_, '_, '_>,
    pb: &ProgressBarData,
    skin: &SliderSkinData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> bool {
    let np = &skin.track;
    let track_h = f32::from(skin.track_h);
    let track_y = y + (h - track_h) / 2.0;

    // Draw track background (9-patch stretched to full width)
    if let Some(bitmap_id) = np.bitmap_id {
        renderer.draw_nine_patch(
            x, track_y, w, track_h, bitmap_id, np.left, np.top, np.right, np.bottom,
        );
    }

    // Draw thumb at progress position (scale down when bar is narrow)
    if let Some(thumb_id) = skin.thumb_id
        && pb.mode == ProgressKind::Slider
    {
        let fraction = pb.fraction.clamp(0.0, 1.0);
        let thumb_w = f32::from(skin.thumb_w);
        let thumb_h = f32::from(skin.thumb_h);
        let scale = (w / (thumb_w * 4.0)).min(1.0);
        let tw = thumb_w * scale;
        let thumb_x = x + fraction * (w - tw);
        let thumb_y = y + (h - thumb_h) / 2.0;
        renderer.draw_bitmap(thumb_x, thumb_y, tw, thumb_h, thumb_id);
    }

    false // skinned bars don't animate (no squiggle)
}

/// Render an animated sine-wave squiggle.
///
/// The squiggle scrolls left via a time-based phase offset, recreating the
/// same visual as the old WASM-side `TranslateX` animation.
fn render_squiggle(
    renderer: &mut dyn Renderer,
    x: f32,
    mid_y: f32,
    width: f32,
    track_h: f32,
    color: Color,
    anim_ctx: &AnimationContext<'_>,
) {
    let amplitude = track_h / 2.0;
    let step = PB_WAVE_LENGTH / PB_WAVE_POINTS_PER_CYCLE as f32;

    // Frame-based scroll offset: one full wavelength per ~50 frames (800ms at 60fps).
    // Use frame_counter alone (not delta_ms) to avoid jitter from variable frame timing.
    let frames_per_cycle = 50.0_f32;
    let phase_frac = (anim_ctx.frame_counter as f32 / frames_per_cycle).fract();
    let offset = -phase_frac * PB_WAVE_LENGTH;

    let start_x = -PB_WAVE_LENGTH + offset;
    let end_x = width + PB_WAVE_LENGTH + offset;
    let n_points = ((end_x - start_x) / step) as usize + 1;

    let points: Vec<(f32, f32)> = (0..n_points)
        .map(|i| {
            let lx = start_x + i as f32 * step;
            let phase = lx / PB_WAVE_LENGTH * std::f32::consts::TAU;
            (x + lx - offset, mid_y + phase.sin() * amplitude)
        })
        .collect();

    renderer.stroke_path(&points, track_h, color, false, true);
}
