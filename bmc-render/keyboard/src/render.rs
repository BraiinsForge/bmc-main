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

//! Keyboard rendering — key grid, visual feedback, layout computation.

use bmc_wasm_protocol::SvgId;
use bmc_wasm_protocol::colors::Color;

use bmc_render::interaction::Rect;

use crate::icons;
use crate::layout::{KeyCode, KeyboardLayout};
use crate::theme::{KeyStyle, resolve_key_style};
use crate::{KeyboardCtx, KeyboardResult};

/// Hold duration before showing popup (ms).
const LONG_PRESS_MS: u32 = 300;

/// Long-press popup state machine.
///
/// `key_id` is `(row, col)` and `popup` references the layout's `&'static str`.
/// No allocations on transition — relevant because they fire each frame.
#[derive(Debug, Default)]
pub(crate) enum LongPressState {
    #[default]
    Idle,
    /// Finger is down, waiting for hold threshold.
    Waiting {
        row: u8,
        col: u8,
        start_ms: u32,
        popup: &'static str,
        /// Key bounds for popup positioning.
        key_x: f32,
        key_y: f32,
        key_w: f32,
        key_h: f32,
    },
    /// Popup is visible, user can slide to select.
    Active {
        popup: &'static str,
        /// Index of currently highlighted character (None = no selection).
        selected: Option<usize>,
        /// Popup bar position and dimensions.
        bar_x: f32,
        bar_y: f32,
        bar_w: f32,
        cell_w: f32,
    },
}

/// Stack buffer for `kb::rRRkCC` keys: `kb::r` (5) + row digits (≤2) + `k` (1)
/// + col digits (≤2) = 10 bytes max. Keyboard grids are well under 100×100.
type KeyIdBuf = [u8; 10];

/// Format a stable key identifier for [`InteractionState`] without allocating.
/// The returned `&str` borrows from the caller's stack `buf`.
///
/// `row` and `col` must be `< 100` so the two-digit ASCII encoding fits in
/// `KeyIdBuf` and produces real digits (out-of-range row/col would emit non-
/// digit ASCII like `:` / `;`, silently mis-matching the InteractionState
/// lookup). Keyboard grids are nowhere near this; the assert just makes
/// the dormant assumption explicit.
#[expect(clippy::integer_division, reason = "writing ASCII digits of u8")]
fn fmt_key_id(buf: &mut KeyIdBuf, row: u8, col: u8) -> &str {
    debug_assert!(
        row < 100 && col < 100,
        "fmt_key_id: row/col must be < 100 (got row={row}, col={col})"
    );
    let mut len = 0;
    for &b in b"kb::r" {
        buf[len] = b;
        len += 1;
    }
    if row >= 10 {
        buf[len] = b'0' + row / 10;
        len += 1;
    }
    buf[len] = b'0' + row % 10;
    len += 1;
    buf[len] = b'k';
    len += 1;
    if col >= 10 {
        buf[len] = b'0' + col / 10;
        len += 1;
    }
    buf[len] = b'0' + col % 10;
    len += 1;
    core::str::from_utf8(&buf[..len]).expect("BUG: fmt_key_id only writes ASCII bytes")
}

/// Scale factor computed from container height vs reference height.
/// All pixel sizes are authored for 480px and multiplied by this.
#[derive(Clone, Copy)]
struct Scale(f32);

impl Scale {
    fn new(container_height: f32) -> Self {
        Self((container_height / REFERENCE_HEIGHT).max(0.6))
    }

    /// Scale a pixel value.
    fn px(self, v: f32) -> f32 {
        v * self.0
    }
}

/// Render a full-screen modal keyboard overlay.
///
/// Returns [`KeyboardResult::Editing`] while the user is typing,
/// [`KeyboardResult::Confirmed`] when they press confirm, or
/// [`KeyboardResult::Cancelled`] when they press cancel.
/// Reference height for scale-factor computation. All sizes are authored
/// for this height and scaled proportionally for smaller/larger containers.
const REFERENCE_HEIGHT: f32 = 480.0;

pub fn render_keyboard(ctx: &mut KeyboardCtx<'_>, layout: &KeyboardLayout) -> KeyboardResult {
    ctx.state.tick(ctx.delta_ms);

    let s = Scale::new(ctx.height);

    // Background
    ctx.renderer
        .fill_rect(0.0, 0.0, ctx.width, ctx.height, ctx.theme.background);

    // Padding and spacing
    let pad = s.px(4.0);
    let inner_w = ctx.width - pad * 2.0;
    let bar_key_gap = s.px(8.0);

    // Layout regions
    let title_height = if ctx.state.title.is_empty() {
        0.0
    } else {
        s.px(28.0)
    };
    let text_bar_height = s.px(40.0);
    let text_bar_y = pad + title_height;
    let key_area_top = text_bar_y + text_bar_height + bar_key_gap;

    // Compute the text-bar layout once and share it with title alignment,
    // input handling, and rendering — single source of truth for the field
    // x-coordinate.
    let bar = text_bar_layout(pad, text_bar_y, text_bar_height, inner_w, s);

    // --- Title (aligned with text input field) ---
    if !ctx.state.title.is_empty() {
        let title_font = s.px(22.0);
        let title_x = bar.text_x + s.px(8.0); // text_padding inside the field
        let title_y = pad + (title_height - title_font * 1.3) / 2.0;
        ctx.renderer.draw_text(
            &ctx.state.title,
            title_x,
            title_y,
            title_font,
            ctx.theme.input.fg,
        );
    }
    let key_area_height = ctx.height - key_area_top - pad;

    // --- Text field bar: update (clicks) first, then draw ---
    if let Some(result) = update_text_bar(ctx, &bar) {
        return result;
    }
    draw_text_bar(ctx, &bar, s);

    // --- Key grid: per-key update + draw stages run inline (single iteration
    //     over the grid; splitting into separate update/draw passes would
    //     double the per-frame work without buying separation that helps a
    //     consumer in practice).
    let rows = layout.active_rows(ctx.state.active_layer, ctx.state.is_shifted());
    render_key_grid(ctx, rows, pad, key_area_top, inner_w, key_area_height, s);

    // --- Long-press popup: update state then draw on top ---
    update_long_press(ctx, s);
    render_long_press_popup(ctx, s);

    // Drain a Confirm signal raised by an in-grid Enter tap.
    if ctx.state.confirm_requested {
        ctx.state.confirm_requested = false;
        return KeyboardResult::Confirmed(ctx.state.text.clone());
    }

    KeyboardResult::Editing
}

/// Float-precision rectangle for layout computation.
#[derive(Clone, Copy)]
struct FRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl FRect {
    /// Convert to integer [`Rect`] for hit testing.
    fn to_rect(self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
        }
    }
}

/// Cancel + Confirm + text-field rectangles for a given text-bar layout.
struct TextBarLayout {
    cancel: FRect,
    confirm: FRect,
    text_x: f32,
    text_width: f32,
    text_h: f32,
    font_size: f32,
}

fn text_bar_layout(x: f32, y: f32, height: f32, width: f32, scale: Scale) -> TextBarLayout {
    let button_width = scale.px(80.0);
    let gap = scale.px(4.0);
    let button_h = height;
    let font_size = scale.px(20.0);
    TextBarLayout {
        cancel: FRect {
            x,
            y,
            w: button_width,
            h: button_h,
        },
        confirm: FRect {
            x: x + width - button_width,
            y,
            w: button_width,
            h: button_h,
        },
        text_x: x + button_width + gap,
        text_width: width - 2.0 * (button_width + gap),
        text_h: font_size * 1.3,
        font_size,
    }
}

/// Process Cancel / Confirm clicks on the text bar.
///
/// Registers both button bounds with `InteractionState` and returns
/// `Some(KeyboardResult)` on click; `None` if neither was clicked this frame.
fn update_text_bar(ctx: &mut KeyboardCtx<'_>, bar: &TextBarLayout) -> Option<KeyboardResult> {
    let (cancel_clicked, _) = ctx
        .interaction
        .button_with_pos("kb::cancel", bar.cancel.to_rect());
    if cancel_clicked {
        return Some(KeyboardResult::Cancelled);
    }
    let (confirm_clicked, _) = ctx
        .interaction
        .button_with_pos("kb::confirm", bar.confirm.to_rect());
    if confirm_clicked {
        return Some(KeyboardResult::Confirmed(ctx.state.text.clone()));
    }
    None
}

/// Render the text bar visuals: Cancel + Confirm buttons, input field bg,
/// the text or placeholder, and the blinking cursor.
fn draw_text_bar(ctx: &mut KeyboardCtx<'_>, bar: &TextBarLayout, scale: Scale) {
    let radius = scale.px(6.0);
    let row_y = bar.cancel.y;
    let row_h = bar.cancel.h;

    // Cancel button
    draw_style_bg(ctx, ctx.theme.cancel, bar.cancel, radius, 0.0);
    let cancel_label_w = ctx.renderer.measure_text("Cancel", bar.font_size);
    ctx.renderer.draw_text(
        "Cancel",
        bar.cancel.x + (bar.cancel.w - cancel_label_w) / 2.0,
        row_y + (row_h - bar.text_h) / 2.0,
        bar.font_size,
        ctx.theme.cancel.fg(),
    );

    // Confirm button
    draw_style_bg(ctx, ctx.theme.confirm, bar.confirm, radius, 0.0);
    let ok_label_w = ctx.renderer.measure_text("OK", bar.font_size);
    ctx.renderer.draw_text(
        "OK",
        bar.confirm.x + (bar.confirm.w - ok_label_w) / 2.0,
        row_y + (row_h - bar.text_h) / 2.0,
        bar.font_size,
        ctx.theme.confirm.fg(),
    );

    // Input field background
    ctx.renderer.fill_rounded_rect(
        bar.text_x,
        row_y,
        bar.text_width,
        row_h,
        radius,
        ctx.theme.input.bg,
    );

    let text_padding = scale.px(8.0);
    let text_display_x = bar.text_x + text_padding;
    let text_display_y = row_y + (row_h - bar.text_h) / 2.0;

    if ctx.state.text.is_empty() {
        ctx.renderer.draw_text(
            &ctx.state.placeholder,
            text_display_x,
            text_display_y,
            bar.font_size,
            ctx.theme.input.placeholder,
        );
    } else {
        ctx.renderer.draw_text(
            &ctx.state.text,
            text_display_x,
            text_display_y,
            bar.font_size,
            ctx.theme.input.fg,
        );
    }

    // Smooth fading cursor blink — triangle wave between 0.3 and 1.0 alpha over 1200ms
    let phase = (ctx.state.blink_ms % 1_200) as u16;
    let t = if phase < 600 {
        f32::from(phase) / 600.0
    } else {
        1.0 - f32::from(phase - 600) / 600.0
    };
    let cursor_alpha = 0.3 + 0.7 * t;
    let cursor_color = ctx.theme.input.cursor.with_alpha(cursor_alpha);

    // cursor is always on a char boundary (maintained by insert_char/backspace)
    #[expect(clippy::string_slice, reason = "cursor tracks char boundaries")]
    let cursor_x = ctx
        .renderer
        .measure_text(&ctx.state.text[..ctx.state.cursor], bar.font_size);
    ctx.renderer.fill_rect(
        text_display_x + cursor_x,
        text_display_y,
        scale.px(2.0),
        bar.text_h,
        cursor_color,
    );
}

/// Render the key grid for the given rows.
fn render_key_grid(
    ctx: &mut KeyboardCtx<'_>,
    rows: &[&[crate::layout::Key]],
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    scale: Scale,
) {
    let row_count = rows.len();
    if row_count == 0 {
        return;
    }
    let gap = scale.px(4.0);
    #[expect(clippy::cast_precision_loss)]
    let row_height = (height - gap * (row_count as f32 - 1.0)) / row_count as f32;

    for (row_idx, row) in rows.iter().enumerate() {
        #[expect(clippy::cast_precision_loss)]
        let row_y = y + (row_height + gap) * row_idx as f32;

        let total_units: f32 = row.iter().map(|k| k.width).sum();
        let key_gap = scale.px(4.0);
        #[expect(clippy::cast_precision_loss)]
        let available_width = width - key_gap * (row.len() as f32 - 1.0);

        // Center the row
        #[expect(clippy::cast_precision_loss)]
        let row_total_width: f32 = row
            .iter()
            .map(|k| k.width / total_units * available_width)
            .sum::<f32>()
            + key_gap * (row.len() as f32 - 1.0);
        let row_x_offset = (width - row_total_width) / 2.0;

        let mut key_x = x + row_x_offset;

        #[expect(clippy::cast_possible_truncation, reason = "row index < 256")]
        let row_u8 = row_idx as u8;
        for (key_idx, key) in row.iter().enumerate() {
            #[expect(clippy::cast_possible_truncation, reason = "col index < 256")]
            let col_u8 = key_idx as u8;

            // ── Layout (shared) ──
            let key_width = key.width / total_units * available_width;
            let mut key_id_buf: KeyIdBuf = [0; 10];
            let key_id = fmt_key_id(&mut key_id_buf, row_u8, col_u8);
            let bounds = FRect {
                x: key_x,
                y: row_y,
                w: key_width,
                h: row_height,
            };

            // ── Update: input → state ──
            let (clicked, _) = ctx.interaction.button_with_pos(key_id, bounds.to_rect());
            let finger_down = ctx.interaction.is_pressed(key_id);
            let long_press_active =
                matches!(ctx.state.long_press, crate::LongPressState::Active { .. });
            // Long-press: start waiting when finger goes down on a popup-bearing key.
            if finger_down
                && !key.popup.is_empty()
                && matches!(ctx.state.long_press, crate::LongPressState::Idle)
            {
                ctx.state.long_press = crate::LongPressState::Waiting {
                    row: row_u8,
                    col: col_u8,
                    start_ms: ctx.state.monotonic_ms,
                    popup: key.popup,
                    key_x,
                    key_y: row_y,
                    key_w: key_width,
                    key_h: row_height,
                };
            }

            // ── Draw: state → pixels ──
            // Inert keys (today: Enter under `EnterBehavior::Disabled`) stay
            // un-highlighted so they don't visually pretend to act on tap.
            let inert = matches!(key.code, KeyCode::Enter)
                && ctx.state.enter_behavior == crate::EnterBehavior::Disabled;
            let highlight = if inert {
                0.0
            } else if finger_down {
                1.0
            } else if ctx.state.last_pressed_key == Some((row_u8, col_u8)) {
                #[expect(clippy::cast_precision_loss, reason = "ms value clamped to 150")]
                let t = ctx
                    .state
                    .monotonic_ms
                    .wrapping_sub(ctx.state.last_release_ms)
                    .min(150) as f32;
                (1.0 - t / 150.0).max(0.0)
            } else {
                0.0
            };
            let (style, hint) = resolve_key_style(
                ctx.theme,
                key.code,
                ctx.state.shift_active,
                ctx.state.caps_lock,
            );
            let fg = draw_style_bg(ctx, style, bounds, scale.px(4.0), highlight);
            render_key_content(ctx, key, bounds, scale, fg, hint);

            // ── Update (post-draw): commit click ──
            // Click is applied here so its highlight lands on the *next* frame's
            // draw — same shape as before the staging refactor. Inert keys (e.g.
            // disabled Enter) skip the highlight write so they don't briefly flash.
            if clicked && !long_press_active && handle_key_press(ctx, key.code) {
                ctx.state.last_pressed_key = Some((row_u8, col_u8));
                ctx.state.last_release_ms = ctx.state.monotonic_ms;
            }

            key_x += key_width + key_gap;
        }
    }
}

/// Render a key's content (icon for special keys, text for characters).
fn render_key_content(
    ctx: &mut KeyboardCtx<'_>,
    key: &crate::layout::Key,
    bounds: FRect,
    scale: Scale,
    fg: Color,
    hint: Color,
) {
    let icon_size = scale.px(28.0);

    // SVG keys: shift/backspace/enter with empty labels get SVG icons.
    // If the label is non-empty (e.g. "=\\<" on symbols layer), render text instead.
    //
    // If icon registration returns `None` (parse failure / ID-space exhaustion)
    // the function key renders blank — the empty `key.label` falls through to
    // the text branch which draws nothing. See `icons::id_for` for the
    // log-once and the BDK-458 follow-up on the registry-lifecycle side.
    let icon: Option<SvgId> = match key.code {
        KeyCode::Shift if key.label.is_empty() => icons::shift_id(ctx.renderer),
        KeyCode::Backspace if key.label.is_empty() => icons::backspace_id(ctx.renderer),
        KeyCode::Enter if key.label.is_empty() => icons::enter_id(ctx.renderer),
        KeyCode::Char(_)
        | KeyCode::Backspace
        | KeyCode::Enter
        | KeyCode::Space
        | KeyCode::Shift
        | KeyCode::SwitchLayer(_)
        | KeyCode::ToggleSubLayer => None,
    };

    if let Some(icon_id) = icon {
        // Dim the Enter icon when its behavior is Disabled — the key still
        // renders so layouts stay consistent, but the dim signals "inert".
        let icon_fg = if matches!(key.code, KeyCode::Enter)
            && ctx.state.enter_behavior == crate::EnterBehavior::Disabled
        {
            fg.with_alpha(0.4)
        } else {
            fg
        };
        ctx.renderer.draw_svg(
            bounds.x + (bounds.w - icon_size) / 2.0,
            bounds.y + (bounds.h - icon_size) / 2.0,
            icon_size,
            icon_size,
            icon_fg,
            icon_id,
            true,
            &[],
        );
    } else {
        let font_size = scale.px(key_font_size(key.code));
        let label_w = ctx.renderer.measure_text(key.label, font_size);
        let label_x = bounds.x + (bounds.w - label_w) / 2.0;
        let label_y = bounds.y + (bounds.h - font_size * 1.3) / 2.0;
        ctx.renderer
            .draw_text(key.label, label_x, label_y, font_size, fg);

        // First popup character as hint in top-right corner.
        // Scale hint with key height so it stays visible at small sizes.
        let hint_size = (bounds.h * 0.35).min(scale.px(20.0));
        if let Some(hint_ch) = key.popup.chars().next() {
            let hint_pad = hint_size * 0.2;
            let mut ch_buf = [0_u8; 4];
            let hint_str: &str = hint_ch.encode_utf8(&mut ch_buf);
            let hint_w = ctx.renderer.measure_text(hint_str, hint_size);
            ctx.renderer.draw_text(
                hint_str,
                bounds.x + bounds.w - hint_w - hint_pad,
                bounds.y + hint_pad,
                hint_size,
                hint,
            );
        }
    }
}

/// Handle a key press by updating keyboard state and playing sound.
///
/// Returns `true` if the press produced any side effect; `false` for
/// inert keys (today: Enter under [`EnterBehavior::Disabled`]). The caller
/// uses this to skip writing highlight state for inert taps.
fn handle_key_press(ctx: &mut KeyboardCtx<'_>, code: KeyCode) -> bool {
    let sound = match code {
        KeyCode::Char(ch) => {
            ctx.state.insert_char(ch);
            crate::KeySound::Standard
        }
        KeyCode::Backspace => {
            ctx.state.backspace();
            crate::KeySound::Delete
        }
        KeyCode::Space => {
            ctx.state.insert_char(' ');
            crate::KeySound::Spacebar
        }
        KeyCode::Enter => match ctx.state.enter_behavior {
            crate::EnterBehavior::Disabled => return false,
            crate::EnterBehavior::InsertNewline => {
                ctx.state.insert_char('\n');
                crate::KeySound::Return
            }
            crate::EnterBehavior::Confirm => {
                ctx.state.confirm_requested = true;
                crate::KeySound::Return
            }
        },
        KeyCode::Shift => {
            ctx.state.toggle_shift();
            crate::KeySound::Standard
        }
        KeyCode::ToggleSubLayer => {
            ctx.state.shift_active = !ctx.state.shift_active;
            crate::KeySound::Standard
        }
        KeyCode::SwitchLayer(layer) => {
            ctx.state.active_layer = layer;
            ctx.state.shift_active = false;
            crate::KeySound::Standard
        }
    };
    ctx.state.blink_ms = 0;
    crate::sound::play(ctx.audio, sound);
    true
}

// ── Style rendering helpers ─────────────────────────────────────────

/// Draw a [`KeyStyle`] background and return the resolved foreground color.
///
/// For [`KeyStyle::Flat`], blends bg → bg_pressed and fg → fg_pressed by `highlight`.
/// For [`KeyStyle::NinePatch`], swaps to pressed bitmap (or darkens normal).
fn draw_style_bg(
    ctx: &mut KeyboardCtx<'_>,
    style: KeyStyle,
    r: FRect,
    radius: f32,
    highlight: f32,
) -> Color {
    match style {
        KeyStyle::Flat {
            bg,
            bg_pressed,
            fg,
            fg_pressed,
            border,
        } => {
            let color = if highlight > 0.0 {
                blend_color(bg, bg_pressed, highlight)
            } else {
                bg
            };
            ctx.renderer
                .fill_rounded_rect(r.x, r.y, r.w, r.h, radius, color);
            if border.alpha() > 0 {
                // TODO: use stroke_rounded_rect once available on Renderer trait
                ctx.renderer.stroke_rect(r.x, r.y, r.w, r.h, 1.0, border);
            }
            if highlight > 0.0 {
                blend_color(fg, fg_pressed, highlight)
            } else {
                fg
            }
        }
        KeyStyle::NinePatch {
            normal,
            pressed,
            fg,
            fg_pressed,
        } => {
            let np = if highlight > 0.5 {
                pressed.unwrap_or(normal)
            } else {
                normal
            };
            if let Some(bitmap_id) = np.bitmap_id {
                ctx.renderer.draw_nine_patch(
                    r.x, r.y, r.w, r.h, bitmap_id, np.left, np.top, np.right, np.bottom,
                );
            }
            if highlight > 0.0 && pressed.is_none() {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "alpha clamped to 0-255"
                )]
                let alpha = (64.0 * highlight) as u8;
                ctx.renderer
                    .fill_rect(r.x, r.y, r.w, r.h, Color::from_rgba(0, 0, 0, alpha));
            }
            if highlight > 0.0 {
                blend_color(fg, fg_pressed, highlight)
            } else {
                fg
            }
        }
    }
}

/// Linear blend between two colors.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "channel values are clamped to 0-255"
)]
fn blend_color(a: Color, b: Color, t: f32) -> Color {
    let mix = |ca: u8, cb: u8| -> u8 {
        let v = f32::from(ca) + (f32::from(cb) - f32::from(ca)) * t;
        v as u8
    };
    Color::from_rgba(
        mix(a.red(), b.red()),
        mix(a.green(), b.green()),
        mix(a.blue(), b.blue()),
        mix(a.alpha(), b.alpha()),
    )
}

/// Font size for a key label (at reference height 480px).
const fn key_font_size(code: KeyCode) -> f32 {
    match code {
        KeyCode::Char(_) => 34.0,
        KeyCode::Space | KeyCode::SwitchLayer(_) | KeyCode::ToggleSubLayer => 22.0,
        KeyCode::Shift | KeyCode::Backspace | KeyCode::Enter => 28.0,
    }
}

// ── Long-press popup ────────────────────────────────────────────────

/// Inner padding of the popup bar as a fraction of cell width.
const POPUP_PAD_RATIO: f32 = 0.10;

/// Update long-press state machine each frame.
fn update_long_press(ctx: &mut KeyboardCtx<'_>, s: Scale) {
    use crate::LongPressState;

    // Check if the finger that started the long-press is still down
    let mut buf: KeyIdBuf = [0; 10];
    let finger_still_down = match &ctx.state.long_press {
        LongPressState::Waiting { row, col, .. } => {
            ctx.interaction.is_pressed(fmt_key_id(&mut buf, *row, *col))
        }
        LongPressState::Active { .. } => {
            // In active state, check if ANY touch is still down (user may have
            // slid off the original key onto the popup bar)
            ctx.interaction.any_touch_down()
        }
        LongPressState::Idle => return,
    };

    if !finger_still_down {
        // Finger released. `mem::take` resets to Idle; only commit if Active.
        if let LongPressState::Active {
            popup, selected, ..
        } = std::mem::take(&mut ctx.state.long_press)
            && let Some(idx) = selected
            && let Some(ch) = popup.chars().nth(idx)
        {
            ctx.state.insert_char(ch);
            ctx.state.blink_ms = 0;
            crate::sound::play(ctx.audio, crate::KeySound::Standard);
        }
        return;
    }

    // Check threshold transition: Waiting → Active
    if let LongPressState::Waiting {
        start_ms,
        popup,
        key_x,
        key_y,
        key_w,
        key_h,
        ..
    } = &ctx.state.long_press
    {
        let elapsed = ctx.state.monotonic_ms.wrapping_sub(*start_ms);
        if elapsed >= LONG_PRESS_MS {
            let char_count = popup.chars().count();
            // Cells are square, sized to match the key height
            let cell_w = *key_h;
            let pad = cell_w * POPUP_PAD_RATIO;
            #[expect(clippy::cast_precision_loss, reason = "popup char count is small")]
            let bar_w = cell_w * char_count as f32 + 2.0 * pad;
            let bar_h = cell_w + 2.0 * pad;
            // Center popup bar above the key; flip below if it would clip the top.
            let bar_x = (key_x + key_w / 2.0 - bar_w / 2.0).max(0.0);
            let gap = s.px(4.0);
            let bar_y_above = key_y - bar_h - gap;
            let bar_y = if bar_y_above >= 0.0 {
                bar_y_above
            } else {
                key_y + *key_h + gap
            };

            ctx.state.long_press = LongPressState::Active {
                popup,
                selected: None,
                bar_x,
                bar_y,
                bar_w,
                cell_w,
            };
        }
    }

    // Update selection based on finger position
    if let LongPressState::Active {
        popup,
        selected,
        bar_x,
        bar_w,
        cell_w,
        ..
    } = &mut ctx.state.long_press
        && let Some((touch_x, _)) = ctx.interaction.last_touch_pos()
    {
        let pad = *cell_w * POPUP_PAD_RATIO;
        let rel_x = touch_x - *bar_x - pad;
        let content_w = *bar_w - 2.0 * pad;
        if rel_x >= 0.0 && rel_x < content_w {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "index from position"
            )]
            let idx = (rel_x / *cell_w) as usize;
            let count = popup.chars().count();
            *selected = if idx < count { Some(idx) } else { None };
        } else {
            *selected = None;
        }
    }
}

/// Render the long-press popup bar (if active).
///
/// The popup uses inner padding so its background is always visible around
/// the character cells, preventing it from blending into the key grid.
/// The selected character gets a circular highlight (Android-style).
fn render_long_press_popup(ctx: &mut KeyboardCtx<'_>, _s: Scale) {
    let crate::LongPressState::Active {
        popup,
        selected,
        bar_x,
        bar_y,
        bar_w,
        cell_w,
    } = &ctx.state.long_press
    else {
        return;
    };

    let pad = *cell_w * POPUP_PAD_RATIO;
    let bar_h = *cell_w + 2.0 * pad;
    let radius = bar_h / 2.0;
    let font_size = *cell_w * 0.55;
    let text_h = font_size * 1.3;

    // Background
    ctx.renderer
        .fill_rounded_rect(*bar_x, *bar_y, *bar_w, bar_h, radius, ctx.theme.popup.bg);

    // Characters with circular selection highlight
    for (i, ch) in popup.chars().enumerate() {
        #[expect(clippy::cast_precision_loss, reason = "small index")]
        let cx = *bar_x + pad + i as f32 * *cell_w;
        let cy = *bar_y + pad;

        let fg = if *selected == Some(i) {
            // Circle highlight behind selected character
            let circle_d = *cell_w * 0.82;
            let circle_r = circle_d / 2.0;
            ctx.renderer.fill_rounded_rect(
                cx + (*cell_w - circle_d) / 2.0,
                cy + (*cell_w - circle_d) / 2.0,
                circle_d,
                circle_d,
                circle_r,
                ctx.theme.popup.selected_bg,
            );
            ctx.theme.popup.selected_fg
        } else {
            ctx.theme.popup.fg
        };

        let mut ch_buf = [0_u8; 4];
        let label: &str = ch.encode_utf8(&mut ch_buf);
        let label_w = ctx.renderer.measure_text(label, font_size);
        ctx.renderer.draw_text(
            label,
            cx + (*cell_w - label_w) / 2.0,
            cy + (*cell_w - text_h) / 2.0,
            font_size,
            fg,
        );
    }
}

#[cfg(test)]
mod measurement_geometry_tests {
    use bmc_render::interaction::InteractionState;
    use bmc_render::renderer::Renderer;
    use bmc_render::renderer::test_support::{DrawnRect, DrawnText, ShapingRecorder};

    use crate::layout::ALL_LAYOUTS;
    use crate::sound::SilentSink;
    use crate::theme::KeyboardTheme;
    use crate::{KeyboardCtx, KeyboardState, render_keyboard};

    const WIDTH: f32 = 1280.0;
    const HEIGHT: f32 = 480.0;
    const TYPED: &str = "hello world";
    const CURSOR: usize = 5;
    /// Positions agree with the measured widths that produced them to well
    /// under a pixel; a shaper disagreement moves them by whole pixels.
    const TOLERANCE: f32 = 0.05;

    fn render_qwerty_us() -> ShapingRecorder {
        let mut recorder = ShapingRecorder::new(WIDTH, HEIGHT);
        let mut state = KeyboardState::new(TYPED, "Wi-Fi Password", "password");
        state.cursor = CURSOR;
        let mut interaction = InteractionState::new();
        let mut audio = SilentSink;
        let layout = ALL_LAYOUTS
            .first()
            .expect("BUG: at least one layout must be generated");
        render_keyboard(
            &mut KeyboardCtx {
                renderer: &mut recorder,
                interaction: &mut interaction,
                state: &mut state,
                audio: &mut audio,
                theme: &KeyboardTheme::CARBON_DARK,
                width: WIDTH,
                height: HEIGHT,
                delta_ms: 16,
            },
            layout,
        );
        recorder
    }

    fn only_draw_of(recorder: &ShapingRecorder, text: &str) -> DrawnText {
        let mut matches = recorder.texts.iter().filter(|t| t.text == text);
        let found = matches
            .next()
            .unwrap_or_else(|| panic!("{text:?} must be drawn"))
            .clone();
        assert!(matches.next().is_none(), "{text:?} must be drawn once");
        found
    }

    /// The key background the drawn `text` sits on.
    fn key_bounds_under(recorder: &ShapingRecorder, text: &DrawnText) -> DrawnRect {
        *recorder
            .rounded_rects
            .iter()
            .find(|r| (r.x..r.x + r.w).contains(&text.x) && (r.y..r.y + r.h).contains(&text.y))
            .unwrap_or_else(|| panic!("{:?} must be drawn on a key background", text.text))
    }

    /// The gap between a key's right edge and its right-aligned popup hint.
    fn hint_right_gap(recorder: &mut ShapingRecorder, label: &str, hint: &str) -> f32 {
        let label_draw = only_draw_of(recorder, label);
        let key = key_bounds_under(recorder, &label_draw);
        let hint_draw = only_draw_of(recorder, hint);
        assert_eq!(
            key_bounds_under(recorder, &hint_draw),
            key,
            "the hint must be drawn on the key it belongs to"
        );
        let hint_w = recorder.measure_text(hint, hint_draw.size);
        key.x + key.w - (hint_draw.x + hint_w)
    }

    #[test]
    fn the_cursor_stands_at_the_shaped_width_of_the_text_before_it() {
        let mut recorder = render_qwerty_us();
        let field = only_draw_of(&recorder, TYPED);
        let before_cursor = TYPED
            .get(..CURSOR)
            .expect("BUG: the cursor must sit on a char boundary");

        let expected_x = field.x + recorder.measure_text(before_cursor, field.size);
        assert!(
            recorder.rects.iter().any(|r| {
                (r.x - expected_x).abs() < TOLERANCE && (r.y - field.y).abs() < TOLERANCE
            }),
            "the cursor must stand at {expected_x} on the text baseline, drew {:?}",
            recorder.rects
        );
    }

    #[test]
    fn a_key_label_is_centered_by_its_shaped_width() {
        let mut recorder = render_qwerty_us();
        let label = only_draw_of(&recorder, "a");
        let key = key_bounds_under(&recorder, &label);

        let label_w = recorder.measure_text(&label.text, label.size);
        let left = label.x - key.x;
        let right = key.x + key.w - (label.x + label_w);
        assert!(
            (left - right).abs() < TOLERANCE,
            "a centered label must leave equal margins, left {left} right {right}"
        );
    }

    #[test]
    fn popup_hints_share_one_right_margin_whatever_they_shape_to() {
        let mut recorder = render_qwerty_us();
        // Two hints of different shaped widths: right alignment must absorb
        // the difference, a left-anchored or width-blind draw could not.
        let narrow = recorder.measure_text("ĵ", 20.0);
        let wide = recorder.measure_text("§", 20.0);
        assert!(
            (narrow - wide).abs() > 1.0,
            "the fixture needs hints of visibly different widths"
        );

        let j_gap = hint_right_gap(&mut recorder, "j", "ĵ");
        let s_gap = hint_right_gap(&mut recorder, "s", "§");
        assert!(
            j_gap > 0.0 && (j_gap - s_gap).abs() < TOLERANCE,
            "hints must be inset from their key's right edge alike, got {j_gap} and {s_gap}"
        );
    }
}
