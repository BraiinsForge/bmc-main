// Copyright (C) 2026  Braiins Systems s.r.o.

//! Notification banner component.

#![expect(clippy::cast_precision_loss)]
#![allow(clippy::wildcard_imports)]

use bmc_wasm_protocol::*;
use taffy::prelude::*;

use crate::renderer::Renderer;
use crate::tree::SpanData;

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
fn notification_accent(kind: u8) -> (Color, u16) {
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
        weight: 600,
        color: GRAY_10,
        ..Default::default()
    }
}

fn notification_subtitle_style() -> TextStyle {
    TextStyle {
        size: 14,
        weight: 400,
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
        AvailableSpace::MinContent => Some(0.0),
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
    renderer: &mut dyn Renderer,
) {
    let (accent, icon_id) = notification_accent(notif.kind);
    render_notification_banner(
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
    icon_id: u16,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    renderer: &mut dyn Renderer,
) {
    // Background
    renderer.fill_rect(x, y, w, h, GRAY_90);

    // Left accent border
    renderer.fill_rect(x, y, NOTIF_BORDER_W, h, accent);

    // Icon — vertically centered with the title line
    let title_line_h = notification_title_style().size as f32 * 1.3;
    let icon_x = x + NOTIF_BORDER_W + NOTIF_PAD;
    let icon_y = y + NOTIF_PAD + (title_line_h - NOTIF_ICON_SIZE) / 2.0;
    renderer.draw_icon(
        icon_x,
        icon_y,
        NOTIF_ICON_SIZE,
        NOTIF_ICON_SIZE,
        accent,
        icon_id,
        false,
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
