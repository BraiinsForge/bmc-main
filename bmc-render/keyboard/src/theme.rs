// Copyright (C) 2026  Braiins Systems s.r.o.

//! Keyboard theming — semantic color tokens with cascading override system.
//!
//! [`KeyboardTheme`] groups visual properties into nested sub-structs:
//! [`InputStyle`], [`PopupStyle`], and [`KeyDefaults`]. Resolution order
//! for key styling:
//!
//! 1. Per-key overrides (`keys.overrides`)
//! 2. Shift state overrides (for the Shift key only)
//! 3. Group defaults (`keys.alpha` vs `keys.fn_keys`)
//!
//! Inheritance between levels is resolved at theme construction time —
//! the render path does a flat lookup with no fallback chains.

use std::borrow::Cow;

use bmc_render_skin::{NinePatch, Skin, SkinEntry};
use bmc_wasm_protocol::colors::Color;

use crate::layout::KeyCode;

// Re-export for convenience — palette colors used in CARBON_DARK.
use bmc_wasm_protocol::colors::{
    BLUE_60, BLUE_70, GRAY_10, GRAY_20, GRAY_30, GRAY_40, GRAY_50, GRAY_60, GRAY_70, GRAY_80,
    GRAY_90, GRAY_100, TRANSPARENT, WHITE,
};

/// Accent color used for confirm button and popup selection highlight.
const CONFIRM_COLOR: Color = Color::from_hex(0x0F_62_FE);

// ---------------------------------------------------------------------------
// Bundled skins
// ---------------------------------------------------------------------------

/// Llama skin — bundled 9-patch assets with a classic media player aesthetic.
pub static LLAMA_SKIN: Skin = bmc_render_macros::include_skin!("assets/skins/llama/");

/// Hyperfuse skin — GMK Hyperfuse-inspired colorway with per-key overrides.
pub static HYPERFUSE_SKIN: Skin = bmc_render_macros::include_skin!("assets/skins/hyperfuse/");

// ---------------------------------------------------------------------------
// KeyStyle
// ---------------------------------------------------------------------------

/// Visual style for a key — either flat colors or 9-patch bitmaps.
///
/// The two variants are mutually exclusive: a key is rendered with solid color
/// fills *or* 9-patch bitmaps, never both.
#[derive(Clone, Copy, Debug)]
pub enum KeyStyle {
    /// Solid color fills with optional border.
    Flat {
        bg: Color,
        bg_pressed: Color,
        fg: Color,
        fg_pressed: Color,
        border: Color,
    },
    /// 9-patch bitmaps for custom keycap art.
    NinePatch {
        normal: NinePatch,
        /// Dedicated pressed bitmap. `None` = darken `normal` with overlay.
        pressed: Option<NinePatch>,
        fg: Color,
        fg_pressed: Color,
    },
}

impl KeyStyle {
    /// Const constructor for the common flat-color case.
    #[must_use]
    pub const fn flat(
        bg: Color,
        bg_pressed: Color,
        fg: Color,
        fg_pressed: Color,
        border: Color,
    ) -> Self {
        Self::Flat {
            bg,
            bg_pressed,
            fg,
            fg_pressed,
            border,
        }
    }

    /// Base foreground color (regardless of variant).
    #[must_use]
    pub const fn fg(self) -> Color {
        match self {
            Self::Flat { fg, .. } | Self::NinePatch { fg, .. } => fg,
        }
    }

    /// Pressed foreground color (regardless of variant).
    #[must_use]
    pub const fn fg_pressed(self) -> Color {
        match self {
            Self::Flat { fg_pressed, .. } | Self::NinePatch { fg_pressed, .. } => fg_pressed,
        }
    }
}

// ---------------------------------------------------------------------------
// Sub-structs
// ---------------------------------------------------------------------------

/// Text input field colors.
#[derive(Clone, Copy, Debug)]
pub struct InputStyle {
    pub bg: Color,
    pub fg: Color,
    pub placeholder: Color,
    pub cursor: Color,
}

/// Long-press popup bar colors.
#[derive(Clone, Copy, Debug)]
pub struct PopupStyle {
    pub bg: Color,
    pub fg: Color,
    pub selected_bg: Color,
    pub selected_fg: Color,
}

/// Per-key style override with its own hint color.
#[derive(Clone, Copy, Debug)]
pub struct KeyOverride {
    pub code: KeyCode,
    pub style: KeyStyle,
    /// Hint character color for this key (inherits from `KeyDefaults::hint` if unset in skin).
    pub hint: Color,
}

/// Key group defaults, shift state overrides, and per-key overrides.
///
/// All inheritance is resolved at construction time. The render path picks
/// from these pre-resolved values — no fallback chains.
#[derive(Clone, Debug)]
pub struct KeyDefaults {
    /// Character keys (`Char(_)`, `Space`).
    pub alpha: KeyStyle,
    /// Function keys (`Shift`, `Backspace`, `Enter`, `SwitchLayer`, `ToggleSubLayer`).
    pub fn_keys: KeyStyle,
    /// Shift key when single-shift is active.
    pub shift_active: KeyStyle,
    /// Shift key when caps lock is engaged.
    pub shift_lock: KeyStyle,
    /// Default popup hint character color (top-right corner of key).
    pub hint: Color,
    /// Individual key overrides checked before group defaults.
    /// `Char` variants are matched case-insensitively for ASCII letters
    /// only — non-ASCII chars compare exactly. The `key_value_<x>_*` skin
    /// keys are parsed as single ASCII chars in practice (Hyperfuse's
    /// WASD/Q recolor), so this matches the realistic input.
    ///
    /// `Cow::Borrowed(&[])` for compile-time const themes (no allocation);
    /// `Cow::Owned(...)` for skin-built themes — owned data, freed when the
    /// theme drops, no `Box::leak`.
    pub overrides: Cow<'static, [KeyOverride]>,
}

// ---------------------------------------------------------------------------
// KeyboardTheme
// ---------------------------------------------------------------------------

/// Complete keyboard visual theme.
///
/// All keyboard rendering reads colors from this struct instead of hardcoded
/// palette constants. The default [`CARBON_DARK`](Self::CARBON_DARK) theme
/// reproduces the original hardcoded look.
#[derive(Clone, Debug)]
pub struct KeyboardTheme {
    /// Full-screen keyboard background.
    pub background: Color,
    /// Text input field.
    pub input: InputStyle,
    /// Cancel button style.
    pub cancel: KeyStyle,
    /// Confirm / OK button style.
    pub confirm: KeyStyle,
    /// Key group defaults, shift overrides, per-key overrides.
    pub keys: KeyDefaults,
    /// Long-press popup bar.
    pub popup: PopupStyle,
}

/// Shorthand for TRANSPARENT.
const T: Color = TRANSPARENT;

impl KeyboardTheme {
    /// Default dark theme matching the original Carbon Design System palette.
    pub const CARBON_DARK: Self = Self {
        background: GRAY_100,
        input: InputStyle {
            bg: GRAY_90,
            fg: GRAY_10,
            placeholder: GRAY_60,
            cursor: GRAY_40,
        },
        cancel: KeyStyle::flat(GRAY_80, GRAY_70, GRAY_10, GRAY_10, T),
        confirm: KeyStyle::flat(CONFIRM_COLOR, CONFIRM_COLOR, GRAY_10, GRAY_10, T),
        keys: KeyDefaults {
            alpha: KeyStyle::flat(GRAY_80, GRAY_50, GRAY_10, GRAY_10, T),
            fn_keys: KeyStyle::flat(GRAY_70, GRAY_50, GRAY_10, GRAY_10, T),
            shift_active: KeyStyle::flat(GRAY_60, GRAY_50, GRAY_10, GRAY_10, T),
            shift_lock: KeyStyle::flat(BLUE_60, GRAY_50, GRAY_10, GRAY_10, T),
            hint: GRAY_50,
            overrides: Cow::Borrowed(&[]),
        },
        popup: PopupStyle {
            bg: GRAY_70,
            fg: GRAY_10,
            selected_bg: CONFIRM_COLOR,
            selected_fg: GRAY_10,
        },
    };

    /// Light theme — white/gray keys on a light background.
    pub const CARBON_LIGHT: Self = Self {
        background: GRAY_20,
        input: InputStyle {
            bg: WHITE,
            fg: GRAY_100,
            placeholder: GRAY_50,
            cursor: GRAY_70,
        },
        cancel: KeyStyle::flat(GRAY_30, GRAY_40, GRAY_100, GRAY_100, T),
        confirm: KeyStyle::flat(CONFIRM_COLOR, BLUE_70, WHITE, WHITE, T),
        keys: KeyDefaults {
            alpha: KeyStyle::flat(WHITE, GRAY_30, GRAY_100, GRAY_100, T),
            fn_keys: KeyStyle::flat(GRAY_30, GRAY_50, GRAY_100, GRAY_100, T),
            shift_active: KeyStyle::flat(GRAY_50, GRAY_60, WHITE, WHITE, T),
            shift_lock: KeyStyle::flat(BLUE_60, GRAY_50, WHITE, WHITE, T),
            hint: GRAY_60,
            overrides: Cow::Borrowed(&[]),
        },
        popup: PopupStyle {
            bg: GRAY_30,
            fg: GRAY_100,
            selected_bg: CONFIRM_COLOR,
            selected_fg: WHITE,
        },
    };
}

// ---------------------------------------------------------------------------
// Skin integration
// ---------------------------------------------------------------------------

/// Build a [`KeyStyle`] from optional 9-patch skin assets, falling back to flat colors.
///
/// If `normal` is `Some`, produces a `NinePatch` style. The asset's `color` field
/// overrides `fg` when non-transparent. Otherwise produces `Flat` with the given colors.
fn style_from_skin(
    normal: Option<SkinEntry>,
    pressed: Option<SkinEntry>,
    fg: Color,
    fg_pressed: Color,
    flat_bg: Color,
    flat_bg_pressed: Color,
) -> KeyStyle {
    if let Some(n) = normal {
        let fg = n.color.or(fg);
        let fg_pressed = pressed.map_or(fg, |p| p.color.or(fg_pressed));
        KeyStyle::NinePatch {
            normal: n.nine_patch,
            pressed: pressed.map(|p| p.nine_patch),
            fg,
            fg_pressed,
        }
    } else {
        KeyStyle::flat(flat_bg, flat_bg_pressed, fg, fg_pressed, T)
    }
}

/// Log skin palette and asset entries at debug level.
fn log_skin(skin: &Skin) {
    tracing::debug!(
        name = skin.name,
        description = skin.description,
        palette_count = skin.palette.len(),
        asset_count = skin.assets.len(),
        "loading keyboard skin"
    );
    for &(name, color) in skin.palette {
        tracing::debug!(
            name,
            color = format!("#{:06X}", color.to_u32() >> 8),
            "  palette"
        );
    }
    for asset in skin.assets {
        tracing::debug!(
            name = asset.name,
            color = format!("#{:08X}", asset.color.to_u32()),
            "  asset"
        );
    }
}

impl KeyboardTheme {
    /// Build a keyboard theme from a [`Skin`].
    ///
    /// Missing entries fall back to [`CARBON_DARK`](Self::CARBON_DARK) defaults.
    /// Inheritance chain (each level inherits unspecified props from the next):
    ///
    /// 1. `key_value_{ch}_{prop}` → `key_group_alpha_{prop}`
    /// 2. `shift_active_{prop}` / `shift_lock_{prop}` → `key_group_fn_{prop}`
    /// 3. `key_group_fn_{prop}` → `key_group_alpha_{prop}`
    /// 4. `key_group_alpha_{prop}` → `CARBON_DARK`
    ///
    /// ## `[palette]` keys (via [`Skin::color_or`])
    ///
    /// - `background` — keyboard background
    /// - `input_bg` / `input_fg` / `input_placeholder` / `input_cursor` — text field
    /// - `key_group_alpha_bg` / `_bg_pressed` / `_fg` / `_fg_pressed` — character keys
    /// - `key_group_alpha_hint` — popup hint character color on keys
    /// - `key_group_fn_bg` / `_bg_pressed` / `_fg` / `_fg_pressed` — function keys
    /// - `shift_active_bg` / `shift_lock_bg` — shift key states
    /// - `cancel_bg` / `_bg_pressed` / `_fg` / `_fg_pressed` — cancel button
    /// - `confirm_bg` / `_bg_pressed` / `_fg` / `_fg_pressed` — confirm button
    /// - `popup_bg` / `popup_fg` / `popup_selected_bg` / `popup_selected_fg`
    /// - `key_value_{ch}_bg` / `_bg_pressed` / `_fg` — per-key overrides
    ///
    /// ## `[assets.*]` entries (via [`Skin::get_nine_patch`])
    ///
    /// - `key` / `key_pressed` — character keys (also fallback for all groups)
    /// - `key_fn` / `key_fn_pressed` — function keys
    /// - `btn_cancel` / `btn_cancel_pressed` — cancel button
    /// - `btn_confirm` / `btn_confirm_pressed` — confirm button
    ///
    /// Each asset's `color` field overrides the palette text color for that element.
    #[must_use]
    pub fn from_skin(skin: &Skin) -> Self {
        log_skin(skin);
        let d = &Self::CARBON_DARK;

        // Palette color lookup shorthand.
        let c = |name, fallback| skin.color_or(name, fallback);

        // -- Alpha group (character keys + space) --
        let base = skin.get_nine_patch("key");
        let base_pressed = skin.get_nine_patch("key_pressed");
        let alpha_fg = c("key_group_alpha_fg", d.keys.alpha.fg());
        let alpha_fg_pressed = c("key_group_alpha_fg_pressed", d.keys.alpha.fg_pressed());
        let alpha_bg = c("key_group_alpha_bg", GRAY_80);
        let alpha_bg_pressed = c("key_group_alpha_bg_pressed", GRAY_50);

        let alpha = style_from_skin(
            base,
            base_pressed,
            alpha_fg,
            alpha_fg_pressed,
            alpha_bg,
            alpha_bg_pressed,
        );

        // -- Fn group (shift, backspace, enter, layer switch) --
        // Inherits from alpha for unspecified props.
        let fn_fg = c("key_group_fn_fg", alpha_fg);
        let fn_fg_pressed = c("key_group_fn_fg_pressed", alpha_fg_pressed);
        let fn_bg = c("key_group_fn_bg", GRAY_70);
        let fn_bg_pressed = c("key_group_fn_bg_pressed", alpha_bg_pressed);

        let fn_keys = style_from_skin(
            skin.get_nine_patch("key_fn").or(base),
            skin.get_nine_patch("key_fn_pressed").or(base_pressed),
            fn_fg,
            fn_fg_pressed,
            fn_bg,
            fn_bg_pressed,
        );

        // -- Cancel / Confirm buttons --
        let cancel = style_from_skin(
            skin.get_nine_patch("btn_cancel").or(base),
            skin.get_nine_patch("btn_cancel_pressed").or(base_pressed),
            c("cancel_fg", alpha_fg),
            c("cancel_fg_pressed", alpha_fg_pressed),
            c("cancel_bg", alpha_bg),
            c("cancel_bg_pressed", alpha_bg_pressed),
        );
        let confirm = style_from_skin(
            skin.get_nine_patch("btn_confirm").or(base),
            skin.get_nine_patch("btn_confirm_pressed").or(base_pressed),
            c("confirm_fg", alpha_fg),
            c("confirm_fg_pressed", alpha_fg_pressed),
            c("confirm_bg", CONFIRM_COLOR),
            c("confirm_bg_pressed", CONFIRM_COLOR),
        );

        // -- Shift state overrides (inherit from fn group) --
        let shift_active = KeyStyle::flat(
            c("shift_active_bg", GRAY_60),
            c("shift_active_bg_pressed", fn_bg_pressed),
            c("shift_active_fg", fn_fg),
            c("shift_active_fg_pressed", fn_fg_pressed),
            T,
        );
        let shift_lock = KeyStyle::flat(
            c("shift_lock_bg", BLUE_60),
            c("shift_lock_bg_pressed", fn_bg_pressed),
            c("shift_lock_fg", fn_fg),
            c("shift_lock_fg_pressed", fn_fg_pressed),
            T,
        );

        let hint = c("key_group_alpha_hint", d.keys.hint);

        Self {
            background: c("background", d.background),
            input: InputStyle {
                bg: c("input_bg", d.input.bg),
                fg: c("input_fg", d.input.fg),
                placeholder: c("input_placeholder", d.input.placeholder),
                cursor: c("input_cursor", d.input.cursor),
            },
            cancel,
            confirm,
            keys: KeyDefaults {
                alpha,
                fn_keys,
                shift_active,
                shift_lock,
                hint,
                overrides: Self::parse_key_overrides(skin, &alpha, hint),
            },
            popup: PopupStyle {
                bg: c("popup_bg", d.popup.bg),
                fg: c("popup_fg", d.popup.fg),
                selected_bg: c("popup_selected_bg", d.popup.selected_bg),
                selected_fg: c("popup_selected_fg", d.popup.selected_fg),
            },
        }
    }

    /// Parse per-key color overrides from palette entries matching `key_value_<char>_bg`.
    ///
    /// For each `key_value_<x>_bg` entry, builds a [`KeyOverride`] with style and hint.
    /// Also reads optional `key_value_<x>_fg`, `key_value_<x>_bg_pressed`, and
    /// `key_value_<x>_hint` (defaults inherit from the alpha group / default hint).
    fn parse_key_overrides(
        skin: &Skin,
        alpha: &KeyStyle,
        default_hint: Color,
    ) -> Cow<'static, [KeyOverride]> {
        let mut overrides = Vec::new();
        let default_fg = alpha.fg();
        let default_fg_pressed = alpha.fg_pressed();
        let default_bg_pressed = match *alpha {
            KeyStyle::Flat { bg_pressed, .. } => bg_pressed,
            KeyStyle::NinePatch { .. } => GRAY_50,
        };

        for &(name, color) in skin.palette {
            let Some(rest) = name.strip_prefix("key_value_") else {
                continue;
            };
            let Some(ch_str) = rest.strip_suffix("_bg") else {
                continue;
            };
            // Single character key overrides only
            let mut chars = ch_str.chars();
            let Some(ch) = chars.next() else { continue };
            if chars.next().is_some() {
                continue;
            }

            let fg = skin.color_or(&format!("key_value_{ch_str}_fg"), default_fg);
            let pressed = skin.color_or(
                &format!("key_value_{ch_str}_bg_pressed"),
                default_bg_pressed,
            );
            let hint = skin.color_or(&format!("key_value_{ch_str}_hint"), default_hint);

            overrides.push(KeyOverride {
                code: KeyCode::Char(ch),
                style: KeyStyle::flat(color, pressed, fg, default_fg_pressed, T),
                hint,
            });
        }

        if overrides.is_empty() {
            return Cow::Borrowed(&[]);
        }

        tracing::debug!(count = overrides.len(), "per-key overrides");
        Cow::Owned(overrides)
    }
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Resolve the visual style and hint color for a key using the theme's cascade:
/// per-key override → shift state → group default.
#[must_use]
pub fn resolve_key_style(
    theme: &KeyboardTheme,
    code: KeyCode,
    shift_active: bool,
    caps_lock: bool,
) -> (KeyStyle, Color) {
    let hint = theme.keys.hint;

    // 1. Per-key overrides — `Char` matched case-insensitively for ASCII
    //    only (non-ASCII compares exactly; see `KeyDefaults::overrides` doc).
    for ov in theme.keys.overrides.iter() {
        let matches = match (ov.code, code) {
            (KeyCode::Char(a), KeyCode::Char(b)) => a.eq_ignore_ascii_case(&b),
            (a, b) => a == b,
        };
        if matches {
            return (ov.style, ov.hint);
        }
    }

    // 2. Shift key state overrides
    if code == KeyCode::Shift {
        if caps_lock {
            return (theme.keys.shift_lock, hint);
        }
        if shift_active {
            return (theme.keys.shift_active, hint);
        }
    }

    // 3. Group default
    let style = match code {
        KeyCode::Char(_) | KeyCode::Space => theme.keys.alpha,
        KeyCode::Shift
        | KeyCode::Backspace
        | KeyCode::Enter
        | KeyCode::SwitchLayer(_)
        | KeyCode::ToggleSubLayer => theme.keys.fn_keys,
    };
    (style, hint)
}
