// Copyright (C) 2026  Braiins Systems s.r.o.

//! Progress bar / slider component.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use bmc_wasm_protocol::{BitmapId, Color};

use crate::renderer::Renderer;
use crate::tree::{AnimationContext, SliderSkinData};

// ── Data ─────────────────────────────────────────────────────────────

/// Host-side progress bar rendering data.
#[derive(Clone, Default, Debug)]
pub(crate) struct ProgressBarData {
    pub track_h: f32,
    /// 0 = Fraction, 1 = Indeterminate
    pub mode: u8,
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
    renderer: &mut dyn Renderer,
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
        render_progress_bar_flat(renderer, pb, x, y, w, anim_ctx)
    }
}

/// Flat (unskinned) progress bar: rects, circles, squiggle.
fn render_progress_bar_flat(
    renderer: &mut dyn Renderer,
    pb: &ProgressBarData,
    x: f32,
    y: f32,
    w: f32,
    anim_ctx: &mut AnimationContext<'_>,
) -> bool {
    let track_h = pb.track_h;
    let dot_radius = track_h * 2.0;
    let bar_height = dot_radius * 2.0 + track_h;
    let half_track = track_h / 2.0;
    let mid_y = y + bar_height / 2.0;
    let is_indeterminate = pb.mode == 1;
    let fraction = pb.fraction.clamp(0.0, 1.0);
    let fill_w = w * fraction;

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

            // Clip rect: hide squiggle past the playhead
            let clip_x = x + fill_w;
            renderer.fill_rect(clip_x, y, w - fill_w + 1.0, bar_height, pb.bg_color);

            // Remaining track after the playhead
            let track_x = clip_x + dot_radius;
            renderer.fill_rect(
                track_x,
                mid_y - half_track,
                (w - fill_w - dot_radius).max(0.0),
                track_h,
                pb.track_color,
            );
            animating = true;
        } else if fill_w > 0.0 {
            // Static fill (not active, or fill too small for squiggle)
            renderer.fill_rect(x, mid_y - half_track, fill_w, track_h, pb.fill_color);
        }

        // Playhead dot — clamp center so it never clips outside the bar
        {
            let dot_cx = (x + fill_w).clamp(x + dot_radius, x + w - dot_radius);
            renderer.fill_circle(dot_cx, mid_y, dot_radius, pb.fill_color);
        }
    }

    animating
}

/// Skinned progress bar: 9-patch track + bitmap thumb.
fn render_progress_bar_skinned(
    renderer: &mut dyn Renderer,
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
    renderer.draw_nine_patch(
        x,
        track_y,
        w,
        track_h,
        np.bitmap_id,
        np.left,
        np.top,
        np.right,
        np.bottom,
    );

    // Draw thumb at progress position (scale down when bar is narrow)
    if skin.thumb_id != BitmapId::NONE && pb.mode == 0 {
        let fraction = pb.fraction.clamp(0.0, 1.0);
        let thumb_w = f32::from(skin.thumb_w);
        let thumb_h = f32::from(skin.thumb_h);
        let scale = (w / (thumb_w * 4.0)).min(1.0);
        let tw = thumb_w * scale;
        let thumb_x = x + fraction * (w - tw);
        let thumb_y = y + (h - thumb_h) / 2.0;
        renderer.draw_bitmap(thumb_x, thumb_y, tw, thumb_h, skin.thumb_id);
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
