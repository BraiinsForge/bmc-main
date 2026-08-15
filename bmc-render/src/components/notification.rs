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

//! Notification banner component.

#![expect(clippy::cast_precision_loss)]
#![allow(clippy::wildcard_imports)]

use bmc_wasm_protocol::*;
use taffy::prelude::*;

use crate::renderer::{RenderTarget, Renderer};
use crate::tree::{SpanData, min_content_paragraph_width};

// ── Constants ────────────────────────────────────────────────────────

const NOTIF_BORDER_W: f32 = 3.0;
const NOTIF_PAD: f32 = 12.0;
const NOTIF_ICON_SIZE: f32 = 16.0;
const NOTIF_ICON_GAP: f32 = 8.0;
/// Left offset from notification edge to text start
const NOTIF_TEXT_LEFT: f32 = NOTIF_BORDER_W + NOTIF_PAD + NOTIF_ICON_SIZE + NOTIF_ICON_GAP;

// ── Data ─────────────────────────────────────────────────────────────

/// Notification data for measurement and rendering
#[derive(Clone, Debug)]
pub(crate) struct NotificationData {
    pub kind: u8,
    pub title: String,
    pub subtitle: String,
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Returns (accent_color, icon_id) for a notification kind byte.
fn notification_accent(kind: u8) -> (Color, SvgId) {
    match kind {
        0 => (RED_60, ICON_ERROR),
        1 => (ORANGE_40, ICON_WARNING),
        2 => (GREEN_40, ICON_SUCCESS),
        _ => (VIOLET_50, ICON_INFO),
    }
}

fn notification_title_style() -> TextStyle {
    TextStyle {
        size: 14,
        weight: FontWeight::SEMIBOLD,
        color: GRAY_10,
        ..Default::default()
    }
}

fn notification_subtitle_style() -> TextStyle {
    TextStyle {
        size: 14,
        weight: FontWeight::REGULAR,
        color: GRAY_50,
        ..Default::default()
    }
}

fn plain_spans(text: &str) -> [SpanData; 1] {
    [SpanData {
        text: text.to_owned(),
        weight: None,
        color: None,
        italic: false,
        underline: false,
        strikethrough: false,
    }]
}

// ── Measurement ──────────────────────────────────────────────────────

pub(crate) fn measure_notification(
    notif: &NotificationData,
    known_dimensions: Size<Option<f32>>,
    available_space: Size<AvailableSpace>,
    renderer: &mut dyn Renderer,
) -> Size<f32> {
    let avail_w = known_dimensions.width.or(match available_space.width {
        AvailableSpace::Definite(w) => Some(w),
        // Min-content: wide enough for the widest single word, so the
        // probe wraps at word boundaries. Probing at width 0 instead
        // breaks per glyph into a tower whose height becomes the
        // min-size floor of every ancestor.
        AvailableSpace::MinContent => {
            let mut word_w = 0.0_f32;
            if !notif.title.is_empty() {
                word_w = word_w.max(min_content_paragraph_width(
                    renderer,
                    &notification_title_style(),
                    &plain_spans(&notif.title),
                ));
            }
            if !notif.subtitle.is_empty() {
                word_w = word_w.max(min_content_paragraph_width(
                    renderer,
                    &notification_subtitle_style(),
                    &plain_spans(&notif.subtitle),
                ));
            }
            Some(word_w + NOTIF_TEXT_LEFT + NOTIF_PAD)
        }
        AvailableSpace::MaxContent => None,
    });
    let text_w = avail_w.map(|w| (w - NOTIF_TEXT_LEFT - NOTIF_PAD).max(0.0));

    let mut text_h = 0.0;
    if !notif.title.is_empty() {
        let spans = plain_spans(&notif.title);
        let (_, h) = renderer.measure_paragraph(&notification_title_style(), &spans, text_w);
        text_h += h;
    }
    if !notif.subtitle.is_empty() {
        if text_h > 0.0 {
            text_h += 2.0;
        }
        let spans = plain_spans(&notif.subtitle);
        let (_, h) = renderer.measure_paragraph(&notification_subtitle_style(), &spans, text_w);
        text_h += h;
    }

    let content_h = text_h.max(NOTIF_ICON_SIZE);
    Size {
        width: known_dimensions.width.unwrap_or(avail_w.unwrap_or(300.0)),
        height: known_dimensions
            .height
            .unwrap_or(NOTIF_PAD * 2.0 + content_h),
    }
}

// ── Rendering ────────────────────────────────────────────────────────

pub(crate) fn render_notification(
    notif: &NotificationData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    renderer: &mut RenderTarget<'_, '_, '_>,
) {
    let (accent, icon_id) = notification_accent(notif.kind);
    render_notification_banner_with_target(
        &notif.title,
        &notif.subtitle,
        accent,
        icon_id,
        x,
        y,
        w,
        h,
        renderer,
    );
}

/// Compute the height of a notification banner for a given width.
pub fn measure_notification_banner(
    title: &str,
    subtitle: &str,
    width: f32,
    renderer: &mut dyn Renderer,
) -> f32 {
    let text_w = (width - NOTIF_TEXT_LEFT - NOTIF_PAD).max(0.0);

    let mut text_h = 0.0;
    if !title.is_empty() {
        let spans = plain_spans(title);
        let (_, h) = renderer.measure_paragraph(&notification_title_style(), &spans, Some(text_w));
        text_h += h;
    }
    if !subtitle.is_empty() {
        if text_h > 0.0 {
            text_h += 2.0;
        }
        let spans = plain_spans(subtitle);
        let (_, h) =
            renderer.measure_paragraph(&notification_subtitle_style(), &spans, Some(text_w));
        text_h += h;
    }

    NOTIF_PAD * 2.0 + text_h.max(NOTIF_ICON_SIZE)
}

/// Render a notification-style banner at a given position.
///
/// This is the shared visual used by both the tree notification node and
/// host-side overlays (e.g. the fuel-limiter dead state).
#[expect(clippy::too_many_arguments)]
pub fn render_notification_banner(
    title: &str,
    subtitle: &str,
    accent: Color,
    icon_id: SvgId,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    renderer: &mut dyn Renderer,
) {
    let mut target = RenderTarget::new(renderer, None);
    render_notification_banner_with_target(
        title,
        subtitle,
        accent,
        icon_id,
        x,
        y,
        w,
        h,
        &mut target,
    );
}

#[expect(clippy::too_many_arguments)]
fn render_notification_banner_with_target(
    title: &str,
    subtitle: &str,
    accent: Color,
    icon_id: SvgId,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    renderer: &mut RenderTarget<'_, '_, '_>,
) {
    // Background
    renderer.fill_rect(x, y, w, h, GRAY_90);

    // Left accent border
    renderer.fill_rect(x, y, NOTIF_BORDER_W, h, accent);

    // Icon — vertically centered with the title line
    let title_line_h = notification_title_style().size as f32 * 1.3;
    let icon_x = x + NOTIF_BORDER_W + NOTIF_PAD;
    let icon_y = y + NOTIF_PAD + (title_line_h - NOTIF_ICON_SIZE) / 2.0;
    renderer.draw_svg(
        icon_x,
        icon_y,
        NOTIF_ICON_SIZE,
        NOTIF_ICON_SIZE,
        accent,
        icon_id,
        false,
        &[],
    );

    // Text
    let text_x = x + NOTIF_TEXT_LEFT;
    let text_w = (w - NOTIF_TEXT_LEFT - NOTIF_PAD).max(0.0);
    let mut text_y = y + NOTIF_PAD;

    if !title.is_empty() {
        let style = notification_title_style();
        let spans = plain_spans(title);
        let (_, th) = renderer.measure_paragraph(&style, &spans, Some(text_w));
        renderer.draw_paragraph(&style, &spans, text_x, text_y, text_w);
        text_y += th + 2.0;
    }
    if !subtitle.is_empty() {
        let style = notification_subtitle_style();
        let spans = plain_spans(subtitle);
        renderer.draw_paragraph(&style, &spans, text_x, text_y, text_w);
    }
}
