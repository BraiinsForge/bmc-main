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

//! Button component with immediate-mode API.

#![allow(clippy::wildcard_imports)]

use crate::colors::*;
use crate::interaction::{InteractionState, Rect};
use crate::renderer::{RenderTarget, Renderer};
use crate::tree::ButtonSkinData;

use bmc_wasm_protocol::SvgId;
use bmc_wasm_protocol::colors::Color;
pub use bmc_wasm_protocol::{ButtonSize, ButtonStyle};

// Button semantic colors (normal, active/pressed)
const BTN_PRIMARY_BG: Color = VIOLET_60;
const BTN_PRIMARY_BG_ACTIVE: Color = VIOLET_70;

const BTN_SECONDARY_BG: Color = GRAY_70;
const BTN_SECONDARY_BG_ACTIVE: Color = GRAY_80;

const BTN_DANGER_BG: Color = RED_60;
const BTN_DANGER_BG_ACTIVE: Color = RED_70;

const BTN_TERTIARY_BG: Color = TRANSPARENT;
const BTN_TERTIARY_BG_ACTIVE: Color = GRAY_50;
const BTN_TERTIARY_BORDER: Color = GRAY_50;

const BTN_GHOST_BG_ACTIVE: Color = GRAY_80;

const BTN_FG: Color = GRAY_10;
const BTN_TERTIARY_FG_ACTIVE: Color = GRAY_100;

/// Get button style colors (normal, active/pressed).
fn style_colors(style: ButtonStyle) -> (Color, Color) {
    match style {
        ButtonStyle::Primary => (BTN_PRIMARY_BG, BTN_PRIMARY_BG_ACTIVE),
        ButtonStyle::Secondary => (BTN_SECONDARY_BG, BTN_SECONDARY_BG_ACTIVE),
        ButtonStyle::Danger => (BTN_DANGER_BG, BTN_DANGER_BG_ACTIVE),
        ButtonStyle::Tertiary => (BTN_TERTIARY_BG, BTN_TERTIARY_BG_ACTIVE),
        ButtonStyle::Ghost => (TRANSPARENT, BTN_GHOST_BG_ACTIVE),
    }
}

/// Get border color for a button style.
fn style_border_color(style: ButtonStyle) -> Color {
    match style {
        ButtonStyle::Tertiary => BTN_TERTIARY_BORDER,
        ButtonStyle::Primary
        | ButtonStyle::Secondary
        | ButtonStyle::Danger
        | ButtonStyle::Ghost => Color::default(),
    }
}

// ── Disabled state colors (Carbon Design System, g100 dark theme) ────

const BTN_DISABLED_BG: Color = GRAY_50.with_alpha(0.3);
const BTN_DISABLED_FG_ON_COLOR: Color = WHITE.with_alpha(0.25);
const BTN_DISABLED_FG: Color = GRAY_10.with_alpha(0.25);

/// Draw a button with optional icon and label, and check if it was clicked.
///
/// - `icon_id == None`: text-only button
/// - `icon_id == Some, label empty`: icon-only button (icon centered)
/// - `icon_id == Some, label present`: icon + text button
///
/// When `disabled` is true, the button renders in a dimmed state per Carbon
/// guidelines and does not register clicks or pressed state.
///
/// When `skin` is `Some`, the button background is a 9-patch bitmap instead of
/// a solid-color fill. The pressed state uses `skin.pressed` if available, otherwise
/// darkens the normal 9-patch — never falls back to solid-color rendering.
///
/// Returns `(clicked, click_position)` where click position is local to the button bounds.
#[expect(clippy::too_many_arguments)]
pub fn draw_button(
    renderer: &mut dyn Renderer,
    interaction: &mut InteractionState,
    key: &str,
    label: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    style: ButtonStyle,
    size: ButtonSize,
    icon_id: Option<SvgId>,
    disabled: bool,
    skin: Option<&ButtonSkinData>,
) -> (bool, Option<(f32, f32)>) {
    let mut target = RenderTarget::new(renderer, None);
    draw_button_with_target(
        &mut target,
        interaction,
        key,
        label,
        x,
        y,
        w,
        h,
        style,
        size,
        icon_id,
        disabled,
        skin,
    )
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn draw_button_with_target(
    renderer: &mut RenderTarget<'_, '_, '_>,
    interaction: &mut InteractionState,
    key: &str,
    label: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    style: ButtonStyle,
    size: ButtonSize,
    icon_id: Option<SvgId>,
    disabled: bool,
    skin: Option<&ButtonSkinData>,
) -> (bool, Option<(f32, f32)>) {
    let bounds = Rect::new(x, y, w, h);

    if disabled {
        renderer.push_scissor(x, y, w, h);
        draw_button_disabled(renderer, label, x, y, w, h, style, size, icon_id, skin);
        renderer.pop_scissor();
        return (false, None);
    }

    let is_pressed = interaction.is_pressed(key);

    // Draw button background
    if let Some(skin) = skin {
        // Skinned: always use 9-patch, never fall back to solid color
        let np = if is_pressed {
            skin.pressed.as_ref().unwrap_or(&skin.normal)
        } else {
            &skin.normal
        };
        if let Some(bitmap_id) = np.bitmap_id {
            renderer.draw_nine_patch(x, y, w, h, bitmap_id, np.left, np.top, np.right, np.bottom);
        }
        // Darken overlay when pressed and no dedicated pressed asset
        if is_pressed && skin.pressed.is_none() {
            renderer.fill_rect(x, y, w, h, BLACK.with_alpha(0.25));
        }
    } else if style.is_ghost() {
        // Ghost: no chrome normally, subtle rectangular fill on press
        if is_pressed {
            let (_, active_color) = style_colors(style);
            renderer.fill_rect(x, y, w, h, active_color);
        }
    } else if style.is_outline() {
        if is_pressed {
            let (_, active_color) = style_colors(style);
            renderer.fill_rect(x, y, w, h, active_color);
        } else {
            renderer.stroke_rect(x, y, w, h, 1.0, style_border_color(style));
        }
    } else {
        let (normal_color, active_color) = style_colors(style);
        let bg_color = if is_pressed {
            active_color
        } else {
            normal_color
        };
        renderer.fill_rect(x, y, w, h, bg_color);
    }

    let fg_color = if let Some(skin) = skin {
        let base = if skin.text_color == TRANSPARENT {
            BTN_FG
        } else {
            skin.text_color
        };
        if is_pressed && skin.pressed_text_color != TRANSPARENT {
            skin.pressed_text_color
        } else {
            base
        }
    } else if style.is_outline() && is_pressed {
        BTN_TERTIARY_FG_ACTIVE
    } else {
        BTN_FG
    };

    if !skin.is_some_and(|s| s.opaque) {
        renderer.push_scissor(x, y, w, h);
        draw_button_content(renderer, label, x, y, w, h, size, icon_id, fg_color);
        renderer.pop_scissor();
    }

    interaction.button_with_pos(key, bounds)
}

/// Draw a disabled button per Carbon Design System guidelines.
///
/// - **Primary / Secondary / Danger**: dimmed solid background + dimmed white text/icon.
/// - **Tertiary / Ghost**: transparent background + dimmed gray-10 text/icon.
/// - **Skinned**: 9-patch with darkened overlay + dimmed text/icon.
#[expect(clippy::too_many_arguments)]
fn draw_button_disabled(
    renderer: &mut RenderTarget<'_, '_, '_>,
    label: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    style: ButtonStyle,
    size: ButtonSize,
    icon_id: Option<SvgId>,
    skin: Option<&ButtonSkinData>,
) {
    let fg_color = if let Some(skin) = skin {
        let np = &skin.normal;
        if let Some(bitmap_id) = np.bitmap_id {
            renderer.draw_nine_patch(x, y, w, h, bitmap_id, np.left, np.top, np.right, np.bottom);
        }
        renderer.fill_rect(x, y, w, h, BLACK.with_alpha(0.50)); // darken overlay for disabled
        if skin.text_color == TRANSPARENT {
            BTN_DISABLED_FG_ON_COLOR
        } else {
            skin.text_color.with_alpha(0.25)
        }
    } else if style.is_ghost() || style.is_outline() {
        BTN_DISABLED_FG
    } else {
        renderer.fill_rect(x, y, w, h, BTN_DISABLED_BG);
        BTN_DISABLED_FG_ON_COLOR
    };

    if !skin.is_some_and(|s| s.opaque) {
        draw_button_content(renderer, label, x, y, w, h, size, icon_id, fg_color);
    }
}

/// Render button content (icon and/or label) centered in the given bounds.
#[expect(clippy::too_many_arguments)]
fn draw_button_content(
    renderer: &mut RenderTarget<'_, '_, '_>,
    label: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    size: ButtonSize,
    icon_id: Option<SvgId>,
    fg_color: Color,
) {
    let font_size = size.font_size();
    let icon_sz = size.icon_size();
    let gap = size.icon_text_gap();

    let has_label = !label.is_empty();
    let pad = size.h_padding();

    match (icon_id, has_label) {
        (Some(icon_id), true) => {
            let icon_x = x + pad;
            let icon_y = y + (h - icon_sz) / 2.0;
            renderer.draw_svg(
                icon_x,
                icon_y,
                icon_sz,
                icon_sz,
                fg_color,
                icon_id,
                false,
                &[],
            );

            let text_h = font_size * 1.3;
            let text_x = icon_x + icon_sz + gap;
            let text_y = y + (h - text_h) / 2.0;
            let max_text_w = w - pad - icon_sz - gap - pad;
            draw_text_ellipsis(
                &mut *renderer,
                label,
                text_x,
                text_y,
                font_size,
                max_text_w,
                fg_color,
            );
        }
        (Some(icon_id), false) => {
            // Icon-only: keep centered
            let icon_x = x + (w - icon_sz) / 2.0;
            let icon_y = y + (h - icon_sz) / 2.0;
            renderer.draw_svg(
                icon_x,
                icon_y,
                icon_sz,
                icon_sz,
                fg_color,
                icon_id,
                false,
                &[],
            );
        }
        (None, _) => {
            let text_h = font_size * 1.3;
            let text_x = x + pad;
            let text_y = y + (h - text_h) / 2.0;
            let max_text_w = w - pad * 2.0;
            draw_text_ellipsis(
                &mut *renderer,
                label,
                text_x,
                text_y,
                font_size,
                max_text_w,
                fg_color,
            );
        }
    }
}

/// Draw text with ellipsis truncation if it exceeds `max_w`.
fn draw_text_ellipsis(
    renderer: &mut dyn Renderer,
    label: &str,
    x: f32,
    y: f32,
    font_size: f32,
    max_w: f32,
    color: Color,
) {
    if max_w <= 0.0 {
        return;
    }
    let text_w = renderer.measure_text(label, font_size);
    // 1px tolerance — measure_text can return slightly different values between calls
    // due to FemtoVG font shaping, which causes false truncation on exact-fit buttons.
    if text_w <= max_w + 1.0 {
        renderer.draw_text(label, x, y, font_size, color);
        return;
    }
    // Binary search for the longest prefix that fits with "…"
    let ellipsis_w = renderer.measure_text("\u{2026}", font_size);
    let target_w = max_w - ellipsis_w;
    if target_w <= 0.0 {
        renderer.draw_text("\u{2026}", x, y, font_size, color);
        return;
    }
    // Find cutoff by character boundary (char_indices yields valid boundaries)
    let mut end = label.len();
    for (i, _) in label.char_indices().rev() {
        let prefix = label
            .get(..i)
            .expect("BUG: char_indices yields valid boundaries");
        let pw = renderer.measure_text(prefix, font_size);
        if pw <= target_w {
            end = i;
            break;
        }
    }
    let mut truncated = String::from(
        label
            .get(..end)
            .expect("BUG: end is from char_indices or len()"),
    );
    truncated.push('\u{2026}');
    renderer.draw_text(&truncated, x, y, font_size, color);
}

#[cfg(test)]
mod truncation_tests {
    use super::*;
    use crate::renderer::test_support::ShapingRecorder;

    const LABEL: &str = "Connect to network";
    const SIZE: ButtonSize = ButtonSize::Normal;

    /// The button whose content box is `slack` wider than the shaped label.
    fn draw_label_in_button(recorder: &mut ShapingRecorder, slack: f32) {
        let label_w = recorder.measure_text(LABEL, SIZE.font_size());
        let mut interaction = InteractionState::new();
        draw_button(
            recorder,
            &mut interaction,
            "btn",
            LABEL,
            0.0,
            0.0,
            label_w + SIZE.h_padding() * 2.0 + slack,
            SIZE.height(),
            ButtonStyle::Primary,
            SIZE,
            None,
            false,
            None,
        );
    }

    #[test]
    fn a_label_the_shaper_says_fits_is_drawn_whole() {
        let mut recorder = ShapingRecorder::default();
        draw_label_in_button(&mut recorder, 1.0);

        assert_eq!(
            recorder.only_text().text,
            LABEL,
            "a label sized from the same shaper the button measures with must not truncate"
        );
    }

    #[test]
    fn a_label_wider_than_its_button_is_truncated_to_fit() {
        let mut recorder = ShapingRecorder::default();
        let font_size = SIZE.font_size();
        let label_w = recorder.measure_text(LABEL, font_size);
        // Half the label's shaped width: wide enough that truncation must keep
        // a prefix, narrow enough that the prefix cannot be the whole label.
        draw_label_in_button(&mut recorder, -label_w / 2.0);

        let drawn = recorder.only_text().text.clone();
        let max_text_w = label_w / 2.0;
        assert!(
            drawn.ends_with('\u{2026}') && LABEL.starts_with(drawn.trim_end_matches('\u{2026}')),
            "an overflowing label must draw as an ellipsized prefix of itself, got {drawn:?}"
        );
        assert!(
            recorder.measure_text(&drawn, font_size) <= max_text_w,
            "the truncated label must shape within the width the button measured against"
        );
    }
}
