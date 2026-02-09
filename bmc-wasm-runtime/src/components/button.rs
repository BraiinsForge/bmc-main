// Copyright (C) 2026  Braiins Systems s.r.o.

//! Button component with immediate-mode API.

#![allow(clippy::wildcard_imports)]
#![expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

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
}

impl From<u32> for ButtonStyle {
    fn from(value: u32) -> Self {
        match value {
            1 => ButtonStyle::Secondary,
            2 => ButtonStyle::Danger,
            3 => ButtonStyle::Tertiary,
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
        }
    }

    fn is_outline(self) -> bool {
        matches!(self, ButtonStyle::Tertiary)
    }

    fn border_color(self) -> u32 {
        match self {
            ButtonStyle::Tertiary => BTN_TERTIARY_BORDER,
            ButtonStyle::Primary | ButtonStyle::Secondary | ButtonStyle::Danger => 0,
        }
    }
}

/// Button font size
const BUTTON_FONT_SIZE: f32 = 16.0;

/// Icon size within buttons
const BUTTON_ICON_SIZE: f32 = 16.0;

/// Gap between icon and text
const ICON_TEXT_GAP: f32 = 8.0;

/// Draw a button with optional icon and label, and check if it was clicked.
///
/// - `icon_id == 0`: text-only button
/// - `icon_id != 0, label empty`: icon-only button (icon centered)
/// - `icon_id != 0, label present`: icon + text button
///
/// Returns `true` the frame the button was clicked (immediate-mode pattern).
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
    icon_id: u16,
) -> bool {
    let bounds = Rect::new(x as i32, y as i32, w as u32, h as u32);
    let is_pressed = interaction.is_pressed(key);

    let (normal_color, active_color) = style.colors();
    let bg_color = if is_pressed {
        active_color
    } else {
        normal_color
    };

    // Draw button background or border
    if style.is_outline() {
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

    let has_icon = icon_id != 0;
    let has_label = !label.is_empty();

    if has_icon && has_label {
        // Icon + text: [icon 16x16] [8px gap] [text], centered together
        let text_w = renderer.measure_text(label, BUTTON_FONT_SIZE);
        let content_w = BUTTON_ICON_SIZE + ICON_TEXT_GAP + text_w;
        let content_x = x + (w - content_w) / 2.0;

        let icon_y = y + (h - BUTTON_ICON_SIZE) / 2.0;
        renderer.draw_icon(
            content_x,
            icon_y,
            BUTTON_ICON_SIZE,
            BUTTON_ICON_SIZE,
            fg_color,
            icon_id,
        );

        let text_h = BUTTON_FONT_SIZE * 1.2;
        let text_x = content_x + BUTTON_ICON_SIZE + ICON_TEXT_GAP;
        let text_y = y + (h - text_h) / 2.0;
        renderer.draw_text(label, text_x, text_y, BUTTON_FONT_SIZE, fg_color);
    } else if has_icon {
        // Icon-only: centered in button
        let icon_x = x + (w - BUTTON_ICON_SIZE) / 2.0;
        let icon_y = y + (h - BUTTON_ICON_SIZE) / 2.0;
        renderer.draw_icon(
            icon_x,
            icon_y,
            BUTTON_ICON_SIZE,
            BUTTON_ICON_SIZE,
            fg_color,
            icon_id,
        );
    } else {
        // Text-only: centered
        let text_w = renderer.measure_text(label, BUTTON_FONT_SIZE);
        let text_h = BUTTON_FONT_SIZE * 1.2;
        let text_x = x + (w - text_w) / 2.0;
        let text_y = y + (h - text_h) / 2.0;
        renderer.draw_text(label, text_x, text_y, BUTTON_FONT_SIZE, fg_color);
    }

    // Register hit region and check for click
    interaction.button(key, bounds)
}
