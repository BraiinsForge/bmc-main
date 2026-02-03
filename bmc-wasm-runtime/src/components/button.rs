// Copyright (C) 2025  Braiins Systems s.r.o.

//! Button component with immediate-mode API.

#![allow(clippy::wildcard_imports)]
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::integer_division
)]

use cosmic_text::{FontSystem, SwashCache};
use tiny_skia::Pixmap;

use crate::color;
use crate::colors::*;
use crate::drawing::shapes::{fill_rect, stroke_rect};
use crate::drawing::text::{draw_text, measure_text};
use crate::interaction::{InteractionState, Rect};

// Button semantic colors (normal, active/pressed)
const BTN_PRIMARY_BG: u32 = VIOLET_60;
const BTN_PRIMARY_BG_ACTIVE: u32 = VIOLET_70;

const BTN_SECONDARY_BG: u32 = GRAY_70;
const BTN_SECONDARY_BG_ACTIVE: u32 = GRAY_80;

const BTN_GHOST_BG: u32 = TRANSPARENT;
const BTN_GHOST_BG_ACTIVE: u32 = color!(GRAY_70, alpha: 0.38);

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
    Ghost = 2,
    Danger = 3,
    Tertiary = 4,
}

impl From<u32> for ButtonStyle {
    fn from(value: u32) -> Self {
        match value {
            1 => ButtonStyle::Secondary,
            2 => ButtonStyle::Ghost,
            3 => ButtonStyle::Danger,
            4 => ButtonStyle::Tertiary,
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
            ButtonStyle::Ghost => (BTN_GHOST_BG, BTN_GHOST_BG_ACTIVE),
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
            ButtonStyle::Primary
            | ButtonStyle::Secondary
            | ButtonStyle::Ghost
            | ButtonStyle::Danger => 0,
        }
    }
}

/// Button font size
const BUTTON_FONT_SIZE: u32 = 16;

/// Draw a button with label and check if it was clicked.
///
/// Returns `true` the frame the button was clicked (immediate-mode pattern).
#[expect(clippy::too_many_arguments)]
pub fn draw_button(
    pixmap: &mut Pixmap,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    interaction: &mut InteractionState,
    key: &str,
    label: &str,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    style: ButtonStyle,
) -> bool {
    let bounds = Rect::new(x, y, w, h);
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
            fill_rect(pixmap, x, y, w, h, bg_color);
        } else {
            stroke_rect(pixmap, x, y, w, h, 1, style.border_color());
        }
    } else {
        fill_rect(pixmap, x, y, w, h, bg_color);
    }

    // Center the label with simple arithmetic
    let text_w = measure_text(font_system, label, BUTTON_FONT_SIZE);
    let text_h = (BUTTON_FONT_SIZE as f32 * 1.2) as u32; // line height

    let text_x = x + (w as i32 - text_w as i32) / 2;
    let text_y = y + (h as i32 - text_h as i32) / 2;

    let fg_color = if style.is_outline() && is_pressed {
        BTN_TERTIARY_FG_ACTIVE
    } else {
        BTN_FG
    };

    draw_text(
        pixmap,
        font_system,
        swash_cache,
        label,
        text_x,
        text_y,
        BUTTON_FONT_SIZE,
        fg_color,
    );

    // Register hit region and check for click
    interaction.button(key, bounds)
}
