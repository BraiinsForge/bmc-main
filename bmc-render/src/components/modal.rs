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

//! Modal dialog overlay component.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
#![allow(clippy::wildcard_imports)]

use std::collections::HashMap;

use taffy::prelude::*;

use bmc_wasm_protocol::*;

use crate::components::{ButtonSize, ButtonStyle, draw_button_with_target};
use crate::interaction::InteractionState;
use crate::renderer::{RenderTarget, Renderer};
use crate::tree::{
    AnimationContext, NodeContext, TouchHit, TreeNode, TreeResult, build_taffy_node,
    compute_taffy_layout, debug_layout_enabled, render_taffy_node,
};
use crate::{ModalState, ScrollState};

/// Collected modal info for overlay rendering
pub(crate) struct ModalInfo {
    pub(crate) modal_id: String,
    pub(crate) is_open: bool,
    pub(crate) padding: u16,
    pub(crate) backdrop_alpha: u8,
    pub(crate) title: String,
    /// Modal body background color. `Color::default()` = default.
    pub(crate) bg_color: Color,
    /// Header background color. `Color::default()` = default.
    pub(crate) header_color: Color,
    /// Title text color. `Color::default()` = default.
    pub(crate) title_color: Color,
    /// Maximum modal width. `0` = no limit.
    pub(crate) max_width: u16,
    pub(crate) body: Vec<TreeNode>,
    pub(crate) footer_primary_key: String,
    pub(crate) footer_primary_label: String,
    pub(crate) footer_secondary_key: String,
    pub(crate) footer_secondary_label: String,
    pub(crate) footer_danger: bool,
}

const MODAL_HEADER_HEIGHT: f32 = 48.0;
const MODAL_ANIMATION_OPEN_MS: f32 = 250.0;
const MODAL_ANIMATION_CLOSE_MS: f32 = 180.0;
/// Default max modal width — half the full viewport (SizeVariant::Medium width).
const MODAL_DEFAULT_MAX_WIDTH: f32 = 638.0;

/// Render a modal overlay
#[expect(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) fn render_modal(
    modal: &ModalInfo,
    width: f32,
    height: f32,
    renderer: &mut RenderTarget<'_, '_, '_>,
    interaction: &mut InteractionState,
    modal_states: &mut HashMap<String, ModalState>,
    scroll_states: &mut HashMap<String, ScrollState>,
    delta_ms: u32,
    result: &mut TreeResult,
    anim_ctx: &mut AnimationContext<'_>,
    taffy: &mut TaffyTree<NodeContext>,
) {
    // Get or create modal state
    let state = modal_states.entry(modal.modal_id.clone()).or_default();

    // Detect state transitions and update animation
    let was_open = state.is_open;
    state.is_open = modal.is_open;

    if modal.is_open && !was_open {
        // Opening: start animation from current progress (or 0)
        // Reset body scroll when opening
        let body_key = format!("{}::body", modal.modal_id);
        if let Some(ss) = scroll_states.get_mut(&body_key) {
            ss.scroll_offset = 0.0;
        }
    } else if !modal.is_open && was_open {
        // Closing: animation will progress towards 0.0
    }

    // Advance animation
    if modal.is_open {
        let delta = delta_ms as f32 / MODAL_ANIMATION_OPEN_MS;
        state.animation_progress = (state.animation_progress + delta).min(1.0);
    } else {
        let delta = delta_ms as f32 / MODAL_ANIMATION_CLOSE_MS;
        state.animation_progress = (state.animation_progress - delta).max(0.0);
    }

    // Skip rendering if fully closed
    if state.animation_progress <= 0.0 {
        return;
    }

    // Easing functions (ease-out for open, ease-in for close)
    let progress = if modal.is_open {
        ease_out(state.animation_progress)
    } else {
        ease_in(state.animation_progress)
    };

    // Draw backdrop (alpha from modal props, scaled by animation progress)
    let backdrop_alpha = ((f32::from(modal.backdrop_alpha) / 255.0) * progress * 255.0) as u8;
    let backdrop_color = Color::from_rgba(0, 0, 0, backdrop_alpha);
    renderer.fill_rect(0.0, 0.0, width, height, backdrop_color);

    // Responsive margin: scale down for small viewports so the modal
    // doesn't drown in padding. At full device size (1280×480) this
    // yields 48 (the SDK DEFAULT_MARGIN). At 160×120 it yields 4.
    let user_padding = f32::from(modal.padding);
    let viewport_min = width.min(height);
    let padding = if viewport_min <= 160.0 {
        4.0_f32.min(user_padding)
    } else if viewport_min <= 300.0 {
        (viewport_min * 0.05).min(user_padding).max(4.0)
    } else {
        user_padding.min(viewport_min * 0.1).max(4.0)
    };

    // Modal content dimensions — adapt to viewport size.
    // Width: capped to max_width (default = medium widget width = 638).
    // Height: capped to full.height (480) so modals don't stretch on large screens.
    let available_width = (width - padding * 2.0).max(0.0);
    let max_w = f32::from(modal.max_width);
    let max_w = if max_w > 0.0 {
        max_w
    } else {
        MODAL_DEFAULT_MAX_WIDTH
    };
    let modal_width = available_width.min(max_w);
    let modal_height = (height.min(480.0) - padding * 2.0).max(0.0);

    // Compact layout for small viewports (half-height widgets ≤ 300px)
    let compact = height <= 300.0;
    let header_height = if compact { 32.0 } else { MODAL_HEADER_HEIGHT };
    let body_padding = if compact { 8.0 } else { 16.0 };
    // Footer height: host renders buttons directly, size adapts to viewport.
    let has_footer = !modal.footer_primary_key.is_empty();
    let footer_btn_size = if compact {
        ButtonSize::Small
    } else {
        ButtonSize::Normal
    };
    let footer_height = if has_footer {
        footer_btn_size.height()
    } else {
        0.0
    };

    // Body height = modal height - header - padding - footer
    // Body fills between header and footer — Scroll node handles its own internal padding.
    let body_height = (modal_height - header_height - footer_height).max(0.0);

    // Animate content position (centered horizontally, slide down from -100px)
    let slide_offset = (1.0 - progress) * -100.0;
    let modal_x = (width - modal_width) / 2.0;
    let modal_y = padding + slide_offset;

    // Content is always fully opaque - only backdrop animates opacity
    // This prevents ugly alpha blending of text over background content

    // Draw modal background (CDS gray100 theme: body and header share GRAY_100)
    let modal_bg = if modal.bg_color == Color::default() {
        GRAY_100
    } else {
        modal.bg_color
    };
    renderer.fill_rect(modal_x, modal_y, modal_width, modal_height, modal_bg);

    // Draw header background (same as body by default per CDS)
    let header_bg = if modal.header_color == Color::default() {
        GRAY_100
    } else {
        modal.header_color
    };
    renderer.fill_rect(modal_x, modal_y, modal_width, header_height, header_bg);

    // Draw header title (single-line, ellipsis-clipped)
    let title_fg = if modal.title_color == Color::default() {
        GRAY_10
    } else {
        modal.title_color
    };
    let title_font_size = if compact { 14.0 } else { 16.0 };
    let title_y_offset = if compact { 7.0 } else { 12.0 };
    let title_x = modal_x + body_padding;
    let title_max_w = modal_width - body_padding - header_height; // leave space for close btn
    {
        let text_w = renderer.measure_text(&modal.title, title_font_size);
        if text_w <= title_max_w + 1.0 {
            renderer.draw_text(
                &modal.title,
                title_x,
                modal_y + title_y_offset,
                title_font_size,
                title_fg,
            );
        } else {
            let ellipsis_w = renderer.measure_text("\u{2026}", title_font_size);
            let target_w = title_max_w - ellipsis_w;
            let mut prefix: &str = "";
            if target_w > 0.0 {
                for (i, _) in modal.title.char_indices().rev() {
                    let candidate = modal
                        .title
                        .get(..i)
                        .expect("BUG: char_indices yields valid boundaries");
                    if renderer.measure_text(candidate, title_font_size) <= target_w {
                        prefix = candidate;
                        break;
                    }
                }
            }
            let truncated = format!("{prefix}\u{2026}");
            renderer.draw_text(
                &truncated,
                title_x,
                modal_y + title_y_offset,
                title_font_size,
                title_fg,
            );
        }
    }

    // Draw close button (X icon in top-right)
    let close_btn_x = modal_x + modal_width - header_height;
    let close_btn_y = modal_y;
    let close_btn_size = header_height;

    let close_key = format!("{}::close", modal.modal_id);

    let (close_was_clicked, _) = draw_button_with_target(
        renderer,
        interaction,
        &close_key,
        "",
        close_btn_x,
        close_btn_y,
        close_btn_size,
        close_btn_size,
        ButtonStyle::Ghost,
        ButtonSize::Normal,
        Some(ICON_CLOSE),
        false,
        None,
    );

    if close_was_clicked {
        result.clicks.insert(
            close_key,
            TouchHit {
                x: 0.0,
                y: 0.0,
                width: close_btn_size,
                height: close_btn_size,
            },
        );
    }

    // Body: wrap children in a Scroll node and let the existing scroll system handle everything.
    // The Scroll renderer auto-adds `scrollbar_clearance` on top of `padding.right`
    // (see `bmc-render/src/tree.rs` Scroll branch), so a uniform `padding` here is
    // the right shape; the right gap stays small.
    let body_scroll_key = format!("{}::body", modal.modal_id);
    let body_scroll = TreeNode::Scroll {
        scroll_key: body_scroll_key,
        props: PropsData {
            gap: 8.0,
            padding: body_padding,
            width: modal_width,
            height: body_height,
            ..PropsData::default()
        },
        children: modal.body.clone(),
    };
    let mut dummy_modals: Vec<ModalInfo> = Vec::new();
    taffy.clear();
    if let Ok(body_root) = build_taffy_node(
        taffy,
        &body_scroll,
        anim_ctx.now_unix_secs,
        result,
        &mut dummy_modals,
    ) {
        let _ = compute_taffy_layout(taffy, body_root, &mut *renderer);
        render_taffy_node(
            taffy,
            body_root,
            modal_x,
            modal_y + header_height,
            renderer,
            interaction,
            scroll_states,
            result,
            anim_ctx,
            0,
        );
    }

    // Render footer buttons directly — host controls sizing.
    // CDS: [secondary | primary] each 50%, or [spacer | primary] if no secondary.
    if has_footer {
        let footer_y = modal_y + modal_height - footer_height;
        let has_secondary = !modal.footer_secondary_key.is_empty();
        let btn_h = footer_height;
        let half_w = modal_width / 2.0;

        let primary_style = if modal.footer_danger {
            ButtonStyle::Danger
        } else {
            ButtonStyle::Primary
        };

        // Secondary button (left half) or empty space
        if has_secondary {
            let (clicked, click_pos) = draw_button_with_target(
                renderer,
                interaction,
                &modal.footer_secondary_key,
                &modal.footer_secondary_label,
                modal_x,
                footer_y,
                half_w,
                btn_h,
                ButtonStyle::Secondary,
                footer_btn_size,
                None,
                false,
                None,
            );
            if clicked {
                result.clicks.insert(
                    modal.footer_secondary_key.clone(),
                    TouchHit {
                        x: click_pos.map_or(0.0, |p| p.0),
                        y: click_pos.map_or(0.0, |p| p.1),
                        width: half_w,
                        height: btn_h,
                    },
                );
            }
        }

        // Primary button (right half)
        let primary_x = modal_x + half_w;
        let (clicked, click_pos) = draw_button_with_target(
            renderer,
            interaction,
            &modal.footer_primary_key,
            &modal.footer_primary_label,
            primary_x,
            footer_y,
            half_w,
            btn_h,
            primary_style,
            footer_btn_size,
            None,
            false,
            None,
        );
        if clicked {
            result.clicks.insert(
                modal.footer_primary_key.clone(),
                TouchHit {
                    x: click_pos.map_or(0.0, |p| p.0),
                    y: click_pos.map_or(0.0, |p| p.1),
                    width: half_w,
                    height: btn_h,
                },
            );
        }
    }

    // Debug layout outlines for the modal frame
    if debug_layout_enabled() {
        // Modal outline
        renderer.stroke_rect(modal_x, modal_y, modal_width, modal_height, 1.0, VIOLET_50);
        // Header outline
        renderer.stroke_rect(modal_x, modal_y, modal_width, header_height, 1.0, BLUE_50);
        // Footer outline
        if has_footer {
            let fy = modal_y + modal_height - footer_height;
            renderer.stroke_rect(modal_x, fy, modal_width, footer_height, 1.0, GREEN_50);
        }
    }
}

/// Ease-out: fast start, slow end
fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// Ease-in: slow start, fast end
fn ease_in(t: f32) -> f32 {
    t.powi(3)
}

#[cfg(test)]
mod title_truncation_tests {
    use super::*;
    use crate::renderer::test_support::ShapingRecorder;

    const TITLE: &str = "Firmware upgrade in progress";
    const TITLE_FONT_SIZE: f32 = 16.0;
    /// The header's title box loses the body padding and the close button.
    const TITLE_CHROME: f32 = 16.0 + MODAL_HEADER_HEIGHT;

    /// The modal whose title box is `slack` wider than the shaped title.
    fn render_titled_modal(recorder: &mut ShapingRecorder, slack: f32) -> f32 {
        let title_w = recorder.measure_text(TITLE, TITLE_FONT_SIZE);
        let modal = ModalInfo {
            modal_id: "modal".to_owned(),
            is_open: true,
            padding: 48,
            backdrop_alpha: 128,
            title: TITLE.to_owned(),
            bg_color: Color::default(),
            header_color: Color::default(),
            title_color: Color::default(),
            max_width: (title_w + TITLE_CHROME + slack).ceil() as u16,
            body: Vec::new(),
            footer_primary_key: String::new(),
            footer_primary_label: String::new(),
            footer_secondary_key: String::new(),
            footer_secondary_label: String::new(),
            footer_danger: false,
        };
        let mut target = RenderTarget::new(recorder, None);
        let mut animation_states = HashMap::new();
        let mut transition_states = HashMap::new();
        render_modal(
            &modal,
            1280.0,
            480.0,
            &mut target,
            &mut InteractionState::new(),
            &mut HashMap::new(),
            &mut HashMap::new(),
            1_000,
            &mut TreeResult::default(),
            &mut AnimationContext {
                animation_states: &mut animation_states,
                transition_states: &mut transition_states,
                delta_ms: 1_000,
                frame_counter: 1,
                draw_counter: 0,
                canvas_index: 0,
                draw_in_canvas: 0,
                mesh_slot_counter: 0,
                has_active: false,
                now_unix_secs: 0,
            },
            &mut TaffyTree::new(),
        );
        f32::from(modal.max_width) - TITLE_CHROME
    }

    #[test]
    fn a_title_the_shaper_says_fits_is_drawn_whole() {
        let mut recorder = ShapingRecorder::default();
        render_titled_modal(&mut recorder, 1.0);

        assert_eq!(
            recorder.only_text().text,
            TITLE,
            "a title sized from the same shaper the header measures with must not truncate"
        );
    }

    #[test]
    fn a_title_wider_than_its_header_is_truncated_to_fit() {
        let mut recorder = ShapingRecorder::default();
        let title_w = recorder.measure_text(TITLE, TITLE_FONT_SIZE);
        let title_max_w = render_titled_modal(&mut recorder, -title_w / 2.0);

        let drawn = recorder.only_text().text.clone();
        assert!(
            drawn.ends_with('\u{2026}') && TITLE.starts_with(drawn.trim_end_matches('\u{2026}')),
            "an overflowing title must draw as an ellipsized prefix of itself, got {drawn:?}"
        );
        assert!(
            recorder.measure_text(&drawn, TITLE_FONT_SIZE) <= title_max_w,
            "the truncated title must shape within the width the header measured against"
        );
    }
}
