// Copyright (C) 2026  Braiins Systems s.r.o.

//! Progress bar / slider builder.

use bmc_render_skin::SliderSkin;
use bmc_wasm_protocol::Color;

use crate::tree::Node;

/// Progress bar mode.
#[derive(Clone, Copy, Debug)]
pub enum ProgressMode {
    /// Known progress as a fraction (0.0–1.0). Shows fill + playhead dot.
    Fraction(f32),
    /// Unknown duration — animated indicator across full width.
    Indeterminate,
}

/// Create a progress bar node.
///
/// - `touch_key`: interaction key for slider drag.
/// - `track_h`: track thickness in pixels (also controls squiggle amplitude).
/// - `mode`: `ProgressMode::Fraction(0.0..=1.0)` or `ProgressMode::Indeterminate`.
/// - `active`: when true, filled portion uses animated squiggle.
/// - `fill_color`: fill, playhead dot, and squiggle color.
/// - `track_color`: background track color.
/// - `bg_color`: used to clip squiggle past the playhead. Pass `0` when not active.
#[expect(clippy::too_many_arguments)]
#[must_use]
pub fn progress_bar(
    touch_key: &str,
    track_h: f32,
    mode: ProgressMode,
    active: bool,
    fill_color: Color,
    track_color: Color,
    bg_color: Color,
    skin: Option<SliderSkin>,
) -> Node {
    Node::ProgressBar {
        touch_key: String::from(touch_key),
        track_h,
        mode,
        active,
        fill_color,
        track_color,
        bg_color,
        skin,
    }
}
