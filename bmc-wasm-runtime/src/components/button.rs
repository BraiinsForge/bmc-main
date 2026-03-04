// Copyright (C) 2026  Braiins Systems s.r.o.

//! Button component with immediate-mode API.

#![allow(clippy::wildcard_imports)]
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use crate::colors::*;
use crate::interaction::{InteractionState, Rect};
use crate::renderer::Renderer;

// Button semantic colors (normal, active/pressed)
const BTN_PRIMARY_BG: u32 = VIOLET_60;
const BTN_PRIMARY_BG_ACTIVE: u32 = VIOLET_70;

const BTN_SECONDARY_BG: u32 = GRAY_70;
const BTN_SECONDARY_BG_ACTIVE: u32 = GRAY_80;

const BTN_DANGER_BG: u32 = RED_60;
const BTN_DANGER_BG_ACTIVE: u32 = RED_70;

const BTN_TERTIARY_BG: u32 = TRANSPARENT;
const BTN_TERTIARY_BG_ACTIVE: u32 = GRAY_50;
const BTN_TERTIARY_BORDER: u32 = GRAY_50;

const BTN_GHOST_BG_ACTIVE: u32 = GRAY_80;

const BTN_FG: u32 = GRAY_10;
const BTN_TERTIARY_FG_ACTIVE: u32 = GRAY_100;

/// Button style variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ButtonStyle {
    Primary = 0,
    Secondary = 1,
    Danger = 2,
    Tertiary = 3,
    /// Transparent background, no border. Pressed state shows a subtle rectangular fill.
    Ghost = 4,
}

impl From<u32> for ButtonStyle {
    fn from(value: u32) -> Self {
        match value {
            1 => ButtonStyle::Secondary,
            2 => ButtonStyle::Danger,
            3 => ButtonStyle::Tertiary,
            4 => ButtonStyle::Ghost,
            _ => ButtonStyle::Primary,
        }
    }
}

/// Colors for button styles (normal, active/pressed).
impl ButtonStyle {
    fn colors(self) -> (u32, u32) {
        match self {
            ButtonStyle::Primary => (BTN_PRIMARY_BG, BTN_PRIMARY_BG_ACTIVE),
            ButtonStyle::Secondary => (BTN_SECONDARY_BG, BTN_SECONDARY_BG_ACTIVE),
            ButtonStyle::Danger => (BTN_DANGER_BG, BTN_DANGER_BG_ACTIVE),
            ButtonStyle::Tertiary => (BTN_TERTIARY_BG, BTN_TERTIARY_BG_ACTIVE),
            ButtonStyle::Ghost => (TRANSPARENT, BTN_GHOST_BG_ACTIVE),
        }
    }

    fn is_outline(self) -> bool {
        matches!(self, ButtonStyle::Tertiary)
    }

    fn is_ghost(self) -> bool {
        matches!(self, ButtonStyle::Ghost)
    }

    fn border_color(self) -> u32 {
        match self {
            ButtonStyle::Tertiary => BTN_TERTIARY_BORDER,
            ButtonStyle::Primary
            | ButtonStyle::Secondary
            | ButtonStyle::Danger
            | ButtonStyle::Ghost => 0,
        }
    }
}

use bmc_wasm_protocol::{BUTTON_SIZE_LARGE, BUTTON_SIZE_SMALL};

/// Button size variants with layout metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonSize {
    Small,
    Normal,
    Large,
}

impl From<u8> for ButtonSize {
    fn from(value: u8) -> Self {
        match value {
            BUTTON_SIZE_SMALL => ButtonSize::Small,
            BUTTON_SIZE_LARGE => ButtonSize::Large,
            _ => ButtonSize::Normal,
        }
    }
}

impl ButtonSize {
    #[must_use]
    pub fn height(self) -> f32 {
        match self {
            ButtonSize::Small => 32.0,
            ButtonSize::Normal => 48.0,
            ButtonSize::Large => 56.0,
        }
    }

    #[must_use]
    pub fn font_size(self) -> f32 {
        match self {
            ButtonSize::Small => 13.0,
            ButtonSize::Normal => 16.0,
            ButtonSize::Large => 18.0,
        }
    }

    #[must_use]
    pub fn icon_size(self) -> f32 {
        match self {
            ButtonSize::Small => 14.0,
            ButtonSize::Normal => 16.0,
            ButtonSize::Large => 20.0,
        }
    }

    #[must_use]
    pub fn h_padding(self) -> f32 {
        match self {
            ButtonSize::Small => 12.0,
            ButtonSize::Normal => 16.0,
            ButtonSize::Large => 20.0,
        }
    }

    #[must_use]
    pub fn icon_text_gap(self) -> f32 {
        match self {
            ButtonSize::Small => 6.0,
            ButtonSize::Normal => 8.0,
            ButtonSize::Large => 10.0,
        }
    }

    /// Compute button width given label length, icon presence.
    #[must_use]
    pub fn width(self, label_len: usize, has_icon: bool) -> f32 {
        let h = self.height();
        if has_icon && label_len == 0 {
            // Icon-only: square
            h
        } else if has_icon {
            let text_w = (label_len as f32 * self.font_size() * 0.5).max(self.font_size() * 2.0);
            self.h_padding() + self.icon_size() + self.icon_text_gap() + text_w + self.h_padding()
        } else {
            let text_w = (label_len as f32 * self.font_size() * 0.5).max(self.font_size() * 3.0);
            text_w + self.h_padding() * 2.0
        }
    }
}

// ── Disabled state colors (Carbon Design System, g100 dark theme) ────

const BTN_DISABLED_BG: u32 = crate::color!(GRAY_50, alpha: 0.3);
const BTN_DISABLED_FG_ON_COLOR: u32 = crate::color!(WHITE, alpha: 0.25);
const BTN_DISABLED_FG: u32 = crate::color!(GRAY_10, alpha: 0.25);

/// Draw a button with optional icon and label, and check if it was clicked.
///
/// - `icon_id == 0`: text-only button
/// - `icon_id != 0, label empty`: icon-only button (icon centered)
/// - `icon_id != 0, label present`: icon + text button
///
/// When `disabled` is true, the button renders in a dimmed state per Carbon
/// guidelines and does not register clicks or pressed state.
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
    icon_id: u16,
    disabled: bool,
) -> (bool, Option<(f32, f32)>) {
    let bounds = Rect::new(x as i32, y as i32, w as u32, h as u32);

    if disabled {
        draw_button_disabled(renderer, label, x, y, w, h, style, size, icon_id);
        return (false, None);
    }

    let is_pressed = interaction.is_pressed(key);

    let (normal_color, active_color) = style.colors();
    let bg_color = if is_pressed {
        active_color
    } else {
        normal_color
    };

    // Draw button background or border
    if style.is_ghost() {
        // Ghost: no chrome normally, subtle rectangular fill on press
        if is_pressed {
            renderer.fill_rect(x, y, w, h, bg_color);
        }
    } else if style.is_outline() {
        if is_pressed {
            renderer.fill_rect(x, y, w, h, bg_color);
        } else {
            renderer.stroke_rect(x, y, w, h, 1.0, style.border_color());
        }
    } else {
        renderer.fill_rect(x, y, w, h, bg_color);
    }

    let fg_color = if style.is_outline() && is_pressed {
        BTN_TERTIARY_FG_ACTIVE
    } else {
        BTN_FG
    };

    draw_button_content(renderer, label, x, y, w, h, size, icon_id, fg_color);

    interaction.button_with_pos(key, bounds)
}

/// Draw a disabled button per Carbon Design System guidelines.
///
/// - **Primary / Secondary / Danger**: dimmed solid background + dimmed white text/icon.
/// - **Tertiary / Ghost**: transparent background + dimmed gray-10 text/icon.
#[expect(clippy::too_many_arguments)]
fn draw_button_disabled(
    renderer: &mut dyn Renderer,
    label: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    style: ButtonStyle,
    size: ButtonSize,
    icon_id: u16,
) {
    let fg_color = if style.is_ghost() || style.is_outline() {
        BTN_DISABLED_FG
    } else {
        renderer.fill_rect(x, y, w, h, BTN_DISABLED_BG);
        BTN_DISABLED_FG_ON_COLOR
    };

    draw_button_content(renderer, label, x, y, w, h, size, icon_id, fg_color);
}

/// Render button content (icon and/or label) centered in the given bounds.
#[expect(clippy::too_many_arguments)]
fn draw_button_content(
    renderer: &mut dyn Renderer,
    label: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    size: ButtonSize,
    icon_id: u16,
    fg_color: u32,
) {
    let font_size = size.font_size();
    let icon_sz = size.icon_size();
    let gap = size.icon_text_gap();

    let has_icon = icon_id != 0;
    let has_label = !label.is_empty();

    if has_icon && has_label {
        let text_w = renderer.measure_text(label, font_size);
        let content_w = icon_sz + gap + text_w;
        let content_x = x + (w - content_w) / 2.0;

        let icon_y = y + (h - icon_sz) / 2.0;
        renderer.draw_icon(content_x, icon_y, icon_sz, icon_sz, fg_color, icon_id);

        let text_h = font_size * 1.3;
        let text_x = content_x + icon_sz + gap;
        let text_y = y + (h - text_h) / 2.0;
        renderer.draw_text(label, text_x, text_y, font_size, fg_color);
    } else if has_icon {
        let icon_x = x + (w - icon_sz) / 2.0;
        let icon_y = y + (h - icon_sz) / 2.0;
        renderer.draw_icon(icon_x, icon_y, icon_sz, icon_sz, fg_color, icon_id);
    } else {
        let text_w = renderer.measure_text(label, font_size);
        let text_h = font_size * 1.3;
        let text_x = x + (w - text_w) / 2.0;
        let text_y = y + (h - text_h) / 2.0;
        renderer.draw_text(label, text_x, text_y, font_size, fg_color);
    }
}
