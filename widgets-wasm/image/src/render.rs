// Copyright (C) 2026  Braiins Systems s.r.o.

//! Image widget rendering — the fitted image, status messages, and stale banner.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

use crate::manifest_params::Sizing;

const ICON_RENEW: Svg = include_svg!("assets/renew.svg");

pub const CONFIGURE_URL: &str = "Set an image URL";
pub const LOADING: &str = "Loading image";
pub const LOAD_FAILED: &str = "Failed to load image";
pub const TOO_LARGE: &str = "Image too large";
pub const BAD_IMAGE: &str = "Unsupported image";

const MESSAGE_TEXT_SIZE: u32 = 24;
const STALE_DATA_TEXT: &str = "Stale data";
const STALE_TEXT_SIZE: u32 = 14;
const STALE_ICON_PX: f32 = 16.0;
const STALE_INSET: f32 = 8.0;
const UPDATING_TEXT: &str = "Updating";

/// Aspect ratio (`w / h`) of an image, defaulting to square on a zero height.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "image dimensions are < 2^24 and exact in f32"
)]
pub fn aspect_of(w: u32, h: u32) -> f32 {
    if h == 0 { 1.0 } else { w as f32 / h as f32 }
}

/// Contain-fit `aspect` into `w`×`h`; returns the centered draw rect.
fn contain(aspect: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
    if aspect >= w / h {
        let dh = w / aspect;
        (0.0, (h - dh) / 2.0, w, dh)
    } else {
        let dw = h * aspect;
        ((w - dw) / 2.0, 0.0, dw, h)
    }
}

/// The image drawn on black: `Contain` letterboxes the fit-within blob; `Cover`
/// fills 1:1 (the host already cropped that blob to the viewport).
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "viewport dimensions are < 2^24 and exact in f32"
)]
pub fn image_view(bitmap: BitmapId, aspect: f32, size: WidgetSize, sizing: Sizing) -> Node {
    let (w, h) = (size.width as f32, size.height as f32);
    let (x, y, dw, dh) = match sizing {
        Sizing::Contain => contain(aspect, w, h),
        Sizing::Cover => (0.0, 0.0, w, h),
    };
    col(
        props!(background: BLACK),
        [canvas(
            props!(flex: 1.0),
            vec![Draw::bitmap_id(x, y, dw, dh, Some(bitmap))],
        )],
    )
}

/// A centered status line (no URL / loading / error).
#[must_use]
pub fn message_view(message: &str, _size: WidgetSize) -> Node {
    col(
        props!(background: BLACK),
        [center(
            props!(flex: 1.0),
            [text(
                message.to_string(),
                style!(
                    size: MESSAGE_TEXT_SIZE,
                    weight: FontWeight::REGULAR,
                    color: GRAY_30,
                    line_height: 1.0
                ),
            )],
        )],
    )
}

fn stale_banner() -> Node {
    row(
        props!(inset_bottom: STALE_INSET, inset_left: STALE_INSET),
        [row(
            props!(
                background: GRAY_100,
                padding: 6.0,
                gap: 6.0,
                cross_align: CrossAlign::Center
            ),
            [
                canvas(
                    props!(width: STALE_ICON_PX, height: STALE_ICON_PX),
                    vec![Draw::svg_builtin(
                        0.0,
                        0.0,
                        STALE_ICON_PX,
                        STALE_ICON_PX,
                        ICON_WARN_FILLED,
                        RED_50,
                    )],
                ),
                text(
                    STALE_DATA_TEXT.to_string(),
                    style!(
                        size: STALE_TEXT_SIZE,
                        weight: FontWeight::BOLD,
                        color: RED_50,
                        line_height: 1.0
                    ),
                ),
            ],
        )],
    )
}

/// Overlay a "stale data" banner onto a column/row root.
#[must_use]
pub fn with_stale_banner(mut root: Node) -> Node {
    if let Node::Column(_, children) | Node::Row(_, children) = &mut root {
        children.push(stale_banner());
    }
    root
}

/// Subtle "updating" pill over the cached image during a background refresh.
fn updating_overlay() -> Node {
    row(
        props!(inset_bottom: STALE_INSET, inset_left: STALE_INSET),
        [row(
            props!(
                background: GRAY_100,
                padding: 6.0,
                cross_align: CrossAlign::Center
            ),
            [text(
                UPDATING_TEXT.to_string(),
                style!(
                    size: STALE_TEXT_SIZE,
                    weight: FontWeight::REGULAR,
                    color: GRAY_30,
                    line_height: 1.0
                ),
            )],
        )],
    )
}

/// Overlay an "updating" pill onto a column/row root.
#[must_use]
pub fn with_updating_overlay(mut root: Node) -> Node {
    if let Node::Column(_, children) | Node::Row(_, children) = &mut root {
        children.push(updating_overlay());
    }
    root
}

// ── Tap-to-reveal menu ───────────────────────────────────────────────

/// Tap-handling keys, shared with the widget.
pub const KEY_TAP: &str = "img_tap";
pub const KEY_RELOAD: &str = "menu_reload";
pub const KEY_CLOSE: &str = "menu_close";

const MENU_WIDTH: f32 = 200.0;
const MENU_GAP: f32 = 8.0;
const MENU_PADDING: f32 = 12.0;

/// Full-bleed transparent catcher (absolute via insets): opens / tap-outside-dismisses.
fn tap_catcher() -> Node {
    touchable(
        KEY_TAP,
        props!(inset_top: 0.0, inset_right: 0.0, inset_bottom: 0.0, inset_left: 0.0),
        Vec::<Draw>::new(),
    )
}

/// The reveal menu — two stacked buttons, absolutely positioned over the image.
fn menu_panel() -> Node {
    center(
        props!(inset_top: 0.0, inset_right: 0.0, inset_bottom: 0.0, inset_left: 0.0),
        [col(
            props!(
                width: MENU_WIDTH,
                gap: MENU_GAP,
                padding: MENU_PADDING,
                background: GRAY_100
            ),
            [
                button!(KEY_RELOAD, "Reload", style: Primary, icon: tree::ensure_registered(&ICON_RENEW)).stretch(),
                button!(KEY_CLOSE, "Close", style: Secondary).stretch(),
            ],
        )],
    )
}

/// Overlay the tap catcher (always) and the menu (when open) onto the root.
#[must_use]
pub fn with_interaction(mut root: Node, menu_open: bool) -> Node {
    if let Node::Column(_, children) | Node::Row(_, children) = &mut root {
        children.push(tap_catcher());
        if menu_open {
            children.push(menu_panel());
        }
    }
    root
}
