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

//! Drawing surface shared by the picture widgets — the fitted picture, status
//! messages, the background-refresh pill and the tap-to-reveal menu.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

use crate::machine::ErrorKind;

const ICON_RENEW: Svg = include_svg!("assets/renew.svg");

pub const LOADING: &str = "Loading image";
pub const LOAD_FAILED: &str = "Failed to load image";
pub const TOO_LARGE: &str = "Image too large";
pub const BAD_IMAGE: &str = "Unsupported image";

/// How the picture fills the viewport.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fit {
    /// Letterbox the whole picture inside the viewport.
    Contain,
    /// Fill the viewport and crop the overflow.
    Cover,
}

impl Fit {
    /// Stable token for the cache identity; a fit change must miss the cache.
    #[must_use]
    pub fn identity_token(self) -> &'static str {
        match self {
            Self::Contain => "contain",
            Self::Cover => "cover",
        }
    }
}

/// Text for a load error — centered when no picture is up, else in the overlay.
#[must_use]
pub fn error_message(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::LoadFailed => LOAD_FAILED,
        ErrorKind::TooLarge => TOO_LARGE,
        ErrorKind::BadImage => BAD_IMAGE,
    }
}

const MESSAGE_TEXT_SIZE: u32 = 24;
const UPDATING_TEXT: &str = "Updating";
const UPDATING_TEXT_SIZE: u32 = 14;

/// Aspect ratio (`w / h`) of a picture, defaulting to square on a zero height.
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

/// The picture drawn on black: `Contain` letterboxes the fit-within blob; `Cover`
/// fills 1:1 (the host already cropped that blob to the viewport).
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "viewport dimensions are < 2^24 and exact in f32"
)]
pub fn image_view(bitmap: BitmapId, aspect: f32, size: WidgetSize, fit: Fit) -> Node {
    let (w, h) = (size.width as f32, size.height as f32);
    let (x, y, dw, dh) = match fit {
        Fit::Contain => contain(aspect, w, h),
        Fit::Cover => (0.0, 0.0, w, h),
    };
    col(
        props!(background: BLACK),
        [canvas(
            props!(flex: 1.0),
            vec![Draw::bitmap_id(x, y, dw, dh, Some(bitmap))],
        )],
    )
}

/// A centered status line (not configured / loading / error).
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

/// The "updating" pill for a background refresh; the SDK `with_overlay` places it.
#[must_use]
pub fn updating_pill() -> Node {
    row(
        props!(
            background: GRAY_100,
            padding: 6.0,
            cross_align: CrossAlign::Center
        ),
        [text(
            UPDATING_TEXT.to_string(),
            style!(
                size: UPDATING_TEXT_SIZE,
                weight: FontWeight::REGULAR,
                color: GRAY_30,
                line_height: 1.0
            ),
        )],
    )
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

/// The reveal menu — two stacked buttons, absolutely positioned over the picture.
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
///
/// Callers that also decorate the picture — a caption, say — must compose that first,
/// so a tap still lands on the catcher rather than on the decoration.
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
