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

#![expect(clippy::cast_possible_truncation)]

//! Skin system — types, registration, and 9-patch parsing.
//!
//! # Architecture
//!
//! A skin is a **generic** collection of named colors and image assets loaded
//! from a zip file (or directory) at compile time via `include_skin!()`.
//!
//! ## skin.toml format
//!
//! ```toml
//! name = "My Skin"
//! description = "A cool skin"
//!
//! # Freeform color palette — any key names the consumer expects.
//! [palette]
//! background = "#151520"
//! popup_fg = "#e0e0ee"
//! accent = "#4a8a4a"
//!
//! # Image assets under [assets.<name>].
//! # Each matches a <name>.9.png or <name>.png file in the skin directory/zip.
//! # Optional `color` field sets a text/icon color associated with the asset.
//! [assets.button]
//! color = "#181828"
//!
//! [assets.button_pressed]
//! color = "#080810"
//! ```
//!
//! ## Widget-agnostic design
//!
//! The skin format makes no assumptions about what the key names mean. Each
//! consumer defines its own vocabulary:
//!
//! - **Media control** reads `button_normal`, `slider_track`, `art_frame`, etc.
//! - **Keyboard** reads `key`, `key_pressed`, `popup_bg`, `input_fg`, etc.
//!
//! Widgets read palette colors via [`Skin::color_or`] and image assets via
//! [`Skin::get_nine_patch`]. Missing entries return `None` / the provided
//! fallback — the consumer falls back to its built-in defaults.
//!
//! This crate is WASM-safe with no heavy dependencies. Image decoding happens
//! in proc macros (build-time only), never at runtime.

use std::cell::RefCell;

use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{BitmapId, StaticAssetSource};

// ---------------------------------------------------------------------------
// Bitmap registrar callback
// ---------------------------------------------------------------------------

type BitmapRegistrar = fn(&str, StaticAssetSource) -> Option<BitmapId>;

thread_local! {
    static BITMAP_REGISTRAR: RefCell<BitmapRegistrar> = const {
        RefCell::new(|_, _| panic!("BUG: skin bitmap registrar not initialized — call bmc_render_skin::init() first"))
    };
}

/// Initialize the skin system with a bitmap registration function. Must be
/// called once before any skin or nine-patch registration.
pub fn init(register_fn: BitmapRegistrar) {
    BITMAP_REGISTRAR.with(|r| *r.borrow_mut() = register_fn);
}

fn register_bitmap(tag: &str, source: StaticAssetSource) -> Option<BitmapId> {
    BITMAP_REGISTRAR.with(|r| r.borrow()(tag, source))
}

// ---------------------------------------------------------------------------
// NinePatchAsset (compile-time)
// ---------------------------------------------------------------------------

/// Compile-time 9-patch descriptor and insets from the `.9.png` border.
///
/// Created by the `include_nine_patch!` proc macro which parses the Android-format
/// `.9.png` at build time and strips the 1px border. WASM builds load the inner
/// bitmap from the widget package; native builds retain embedded bytes.
pub struct NinePatchAsset {
    pub source: StaticAssetSource,
    /// Stable, unique-per-host registration tag (e.g. `"<crate>::<file_stem>"`).
    pub name: &'static str,
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

// ---------------------------------------------------------------------------
// NinePatch (runtime)
// ---------------------------------------------------------------------------

/// Registered 9-patch element — bitmap ID + insets defining stretchable regions.
///
/// Created at runtime from a [`NinePatchAsset`] (compile-time) via
/// [`ensure_nine_patch_registered`], or directly via [`NinePatch::from_id`].
#[derive(Clone, Copy, Debug)]
pub struct NinePatch {
    pub bitmap_id: Option<BitmapId>,
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

impl NinePatch {
    /// Create a `NinePatch` from a pre-registered bitmap ID and explicit insets.
    #[must_use]
    pub fn from_id(bitmap_id: BitmapId, left: u16, top: u16, right: u16, bottom: u16) -> Self {
        Self {
            bitmap_id: Some(bitmap_id),
            left,
            top,
            right,
            bottom,
        }
    }
}

// ---------------------------------------------------------------------------
// NinePatchAsset registration
// ---------------------------------------------------------------------------

/// Register a 9-patch asset with the host and return a `NinePatch`.
///
/// Idempotent: the host dedups by `asset.name`, so a second call returns the
/// cached `BitmapId` without re-decoding/re-uploading.
#[must_use]
pub fn ensure_nine_patch_registered(asset: &NinePatchAsset) -> NinePatch {
    NinePatch {
        bitmap_id: register_bitmap(asset.name, asset.source),
        left: asset.left,
        top: asset.top,
        right: asset.right,
        bottom: asset.bottom,
    }
}

// ---------------------------------------------------------------------------
// SkinAsset + Skin
// ---------------------------------------------------------------------------

/// A single asset within a [`Skin`] — compile-time image data with optional 9-patch insets.
///
/// Plain `.png` files become assets with zero insets (no stretching).
/// `.9.png` files have their insets parsed at build time from the 1px border.
pub struct SkinAsset {
    pub name: &'static str,
    pub source: StaticAssetSource,
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
    /// Bitmap dimensions (pixels). For 9-patches, this is the inner image (border stripped).
    pub width: u16,
    pub height: u16,
    /// Foreground/content color for this asset. `TRANSPARENT` = no override.
    pub color: Color,
}

/// A skin — a named collection of colors and image assets loaded at compile time.
///
/// Created by the `include_skin!` proc macro from a directory or zip file containing
/// a `skin.toml` and optional image files.
///
/// The skin format is **consumer-agnostic** — the consumer defines which palette keys
/// and asset names it looks for. See the [crate-level docs](crate) for details.
///
/// ## skin.toml format
///
/// ```toml
/// name = "My Skin"
/// description = "A cool skin"
///
/// [palette]
/// background = "#151520"     # any key names the consumer expects
/// popup_fg = "#e0e0ee"
///
/// [assets.button]            # matches button.9.png or button.png
/// color = "#181828"          # optional text/icon color for this asset
///
/// [assets.button_pressed]
/// color = "#080810"
/// ```
pub struct Skin {
    /// Human-readable skin name (from `skin.toml`).
    pub name: &'static str,
    /// Short description of the skin (from `skin.toml`).
    pub description: &'static str,
    /// Generic color palette — freeform string→Color map.
    /// Widgets read keys they care about via [`get_color`](Self::get_color).
    pub palette: &'static [(&'static str, Color)],
    /// Image assets (9-patch or plain bitmaps).
    pub assets: &'static [SkinAsset],
}

/// A resolved skin asset — registered 9-patch + metadata from `skin.toml`.
#[derive(Clone, Copy, Debug)]
pub struct SkinEntry {
    pub nine_patch: NinePatch,
    /// Bitmap dimensions (pixels). For 9-patches, this is the inner image (border stripped).
    pub width: u16,
    pub height: u16,
    /// Foreground/content color. `TRANSPARENT` = no override.
    pub color: Color,
}

impl Skin {
    // -- Palette (colors) --

    /// Look up a palette color by name.
    ///
    /// Returns `None` only when the key is absent. A palette entry whose
    /// value is [`Color::TRANSPARENT`] returns `Some(TRANSPARENT)` — callers
    /// that need a fallback should use [`color_or`](Self::color_or).
    #[must_use]
    pub fn get_color(&self, name: &str) -> Option<Color> {
        self.palette
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, c)| *c)
    }

    /// Look up a palette color by name, returning `fallback` if missing.
    #[must_use]
    pub fn color_or(&self, name: &str, fallback: Color) -> Color {
        self.get_color(name).unwrap_or(fallback)
    }

    // -- Assets (9-patches / bitmaps) --

    /// Look up a 9-patch asset by name and register it with the host if needed.
    ///
    /// Registration is idempotent host-side: the tag combines the skin name and
    /// the asset name so two skins with the same asset name don't alias.
    #[must_use]
    pub fn get_nine_patch(&self, name: &str) -> Option<SkinEntry> {
        let asset = self.assets.iter().find(|a| a.name == name)?;
        let tag = format!("skin::{}::{}", self.name, asset.name);
        Some(SkinEntry {
            nine_patch: NinePatch {
                bitmap_id: register_bitmap(&tag, asset.source),
                left: asset.left,
                top: asset.top,
                right: asset.right,
                bottom: asset.bottom,
            },
            width: asset.width,
            height: asset.height,
            color: asset.color,
        })
    }

    /// Get the preview thumbnail asset, if this skin includes a `preview.png`.
    #[must_use]
    pub fn preview(&self) -> Option<SkinEntry> {
        self.get_nine_patch("preview")
    }
}

// ---------------------------------------------------------------------------
// ButtonSkin
// ---------------------------------------------------------------------------

/// Optional skin override for a button.
///
/// When present, the host renders the button with 9-patch backgrounds instead of
/// solid-color fills. Pressed state falls back to normal (with host-side darkening)
/// when `pressed` is `None` — never falls back to the default solid-color style
/// once a skin is active.
#[derive(Clone, Copy, Debug)]
pub struct ButtonSkin {
    pub normal: NinePatch,
    pub pressed: Option<NinePatch>,
    /// Text/icon color for normal state. Default = use button style default.
    pub text_color: Color,
    /// Text/icon color for pressed state. Default = use `text_color`.
    pub pressed_text_color: Color,
    /// When true, the bitmap already contains the visual content (e.g. Winamp
    /// transport icons baked into the sprite). The host skips rendering any
    /// icon or label on top.
    pub opaque: bool,
}

// ---------------------------------------------------------------------------
// SliderSkin
// ---------------------------------------------------------------------------

/// Optional skin override for a progress bar / slider.
///
/// When present, the host renders a 9-patch track background and a bitmap
/// thumb at the progress position. Without a skin, the default flat
/// rect + circle rendering is used.
#[derive(Clone, Copy, Debug)]
pub struct SliderSkin {
    /// Stretchable track background.
    pub track: NinePatch,
    /// Track nine-patch height (pixels) — needed for layout.
    pub track_h: u16,
    /// Thumb bitmap (non-9-patch, fixed size). `None` = no thumb.
    pub thumb_id: Option<BitmapId>,
    pub thumb_w: u16,
    pub thumb_h: u16,
    /// Pressed thumb bitmap. `None` = use `thumb_id` with host-side darkening.
    pub thumb_pressed_id: Option<BitmapId>,
}

// ---------------------------------------------------------------------------
// 9-patch parsing utility (used by proc macros)
// ---------------------------------------------------------------------------

/// Insets parsed from a `.9.png` border — distances from each edge to the stretchable region.
#[derive(Clone, Copy, Debug)]
pub struct NinePatchInsets {
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

/// Parse 9-patch insets from a decoded image's 1px border.
///
/// Pure pixel logic — the caller decodes the PNG and provides dimensions + pixel accessor.
/// `get_pixel` takes `(x, y)` and returns `[r, g, b, a]`.
///
/// Returns `Err` with a descriptive message if the image is too small or
/// has no stretch markers on the top row or left column.
pub fn try_parse_nine_patch_insets(
    w: u32,
    h: u32,
    get_pixel: impl Fn(u32, u32) -> [u8; 4],
) -> Result<NinePatchInsets, String> {
    if w < 3 || h < 3 {
        return Err(format!(".9.png must be at least 3x3 pixels, got {w}x{h}"));
    }
    let h_run = scan_run(w, |x| get_pixel(x, 0))
        .ok_or_else(|| ".9.png top row has no black stretch marker".to_string())?;
    let v_run = scan_run(h, |y| get_pixel(0, y))
        .ok_or_else(|| ".9.png left column has no black stretch marker".to_string())?;

    let inner_w = w - 2;
    let inner_h = h - 2;
    Ok(NinePatchInsets {
        left: (h_run.0 - 1) as u16,
        right: (inner_w - (h_run.1 - 1)) as u16,
        top: (v_run.0 - 1) as u16,
        bottom: (inner_h - (v_run.1 - 1)) as u16,
    })
}

/// Panicking wrapper around [`try_parse_nine_patch_insets`] for use in proc macros
/// where errors are unrecoverable.
#[must_use]
pub fn parse_nine_patch_insets(
    w: u32,
    h: u32,
    get_pixel: impl Fn(u32, u32) -> [u8; 4],
) -> NinePatchInsets {
    try_parse_nine_patch_insets(w, h, get_pixel).expect("BUG: invalid .9.png")
}

fn is_black(px: [u8; 4]) -> bool {
    px[0] == 0 && px[1] == 0 && px[2] == 0 && px[3] == 255
}

/// Scan for a contiguous run of black pixels in range `1..size-1`.
/// Returns `Some((start, end))` or `None` if no black pixel found.
fn scan_run(size: u32, get_pixel: impl Fn(u32) -> [u8; 4]) -> Option<(u32, u32)> {
    let mut start = 0;
    let mut end = 0;
    let mut found = false;
    for i in 1..size - 1 {
        if is_black(get_pixel(i)) {
            if !found {
                start = i;
                found = true;
            }
            end = i + 1;
        }
    }
    found.then_some((start, end))
}

// ---------------------------------------------------------------------------
// Color parsing (used by proc macros)
// ---------------------------------------------------------------------------

/// Parse a hex color string (`"#RRGGBB"` or `"RRGGBB"`) into an opaque [`Color`].
///
/// # Panics
///
/// Panics if the string is not exactly 6 hex digits (after optional `#` prefix).
#[must_use]
pub fn parse_hex_color(hex: &str, context: &str) -> Color {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    assert_eq!(
        hex.len(),
        6,
        "{context}: color must be 6 hex digits, got \"{hex}\""
    );
    let v = u32::from_str_radix(hex, 16)
        .unwrap_or_else(|e| panic!("{context}: invalid hex color \"{hex}\": {e}"));
    Color::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}
