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

//! Modal on-screen keyboard rendered via [`bmc_render::renderer::Renderer`].
//!
//! See `README.md` next to this crate for usage, theming, layout pipeline,
//! and the device-integration roadmap.

mod icons;
pub mod layout;
mod render;
pub mod theme;

pub mod sound;

pub use layout::{Key, KeyCode, KeyboardLayout, LayerId};
pub use render::render_keyboard;
pub use sound::{AudioSink, KeySound, SoundTag};
pub use theme::{InputStyle, KeyDefaults, KeyOverride, KeyStyle, KeyboardTheme, PopupStyle};

use bmc_render::interaction::InteractionState;
use bmc_render::renderer::Renderer;

/// All mutable context needed by [`render_keyboard`].
///
/// Bundles the renderer, interaction state, keyboard state, and timing
/// to avoid a long parameter list on the render function.
///
/// **Single-touch assumption.** Long-press transitions key off `key_id`
/// (which key the press started on), not a touch identifier. On multi-touch
/// hardware a lift-and-retouch on a different key could abort `Waiting`
/// based on a stale signal. The Deck is single-touch, so this is fine —
/// revisit if the keyboard ever ships on multi-touch hardware.
#[expect(missing_debug_implementations, reason = "dyn Renderer is not Debug")]
pub struct KeyboardCtx<'a> {
    pub renderer: &'a mut dyn Renderer,
    pub interaction: &'a mut InteractionState,
    pub state: &'a mut KeyboardState,
    pub audio: &'a mut dyn AudioSink,
    pub theme: &'a KeyboardTheme,
    pub width: f32,
    pub height: f32,
    pub delta_ms: u32,
}

/// What the in-grid Enter key does on tap.
///
/// The default is [`Disabled`](Self::Disabled): the key still renders but
/// does nothing on tap and is drawn dimmed so the user can see it's inert.
/// The host opts in to a real behavior.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EnterBehavior {
    /// Tap is a no-op; key is drawn dimmed. Suits hosts whose only confirm
    /// affordance is the OK button (e.g. password input today).
    #[default]
    Disabled,
    /// Tap inserts a newline at the cursor. Suits multi-line text input.
    InsertNewline,
    /// Tap signals confirmation; `render_keyboard` returns
    /// [`KeyboardResult::Confirmed`] like the OK button.
    Confirm,
}

/// Persistent state for an active keyboard session.
#[derive(Debug)]
pub struct KeyboardState {
    /// Title shown above the text bar — always visible so the user knows
    /// what they're typing for (e.g. "Wi-Fi Password", "Device Name").
    pub title: String,
    /// Placeholder shown in the text field when empty.
    pub placeholder: String,
    /// Current text buffer.
    pub text: String,
    /// Cursor position (byte offset into `text`).
    pub cursor: usize,
    /// Currently active layout layer.
    pub active_layer: LayerId,
    /// Whether shift is active (single press, not caps lock).
    pub shift_active: bool,
    /// Whether caps lock is engaged.
    pub caps_lock: bool,
    /// Monotonic time accumulator for cursor blink (ms). Reset on key press.
    pub blink_ms: u32,
    /// Monotonic clock that never resets (ms). Used for double-tap detection.
    monotonic_ms: u32,
    /// Timestamp (monotonic_ms) of the last shift press, for double-tap
    /// caps lock detection. `None` means "no prior tap on record" — distinct
    /// from a tap at `monotonic_ms == 0` which would otherwise be a sentinel
    /// collision at startup.
    last_shift_ms: Option<u32>,
    /// Last pressed key position (row, col), or `None` on fresh construction.
    /// Drives the post-release highlight decay.
    pub(crate) last_pressed_key: Option<(u8, u8)>,
    /// Monotonic timestamp when the last key was released.
    pub(crate) last_release_ms: u32,
    /// Long-press popup state.
    pub(crate) long_press: LongPressState,
    /// What the in-grid Enter key does on tap.
    pub enter_behavior: EnterBehavior,
    /// Set by `handle_key_press` when Enter is tapped under
    /// `EnterBehavior::Confirm`. Drained by `render_keyboard` to emit
    /// [`KeyboardResult::Confirmed`].
    pub(crate) confirm_requested: bool,
}

use crate::render::LongPressState;

impl KeyboardState {
    #[must_use]
    pub fn new(initial_text: &str, title: &str, placeholder: &str) -> Self {
        Self {
            title: title.to_owned(),
            placeholder: placeholder.to_owned(),
            cursor: initial_text.len(),
            text: initial_text.to_owned(),
            active_layer: LayerId::Letters,
            shift_active: false,
            caps_lock: false,
            blink_ms: 0,
            monotonic_ms: 0,
            last_shift_ms: None,
            last_pressed_key: None,
            last_release_ms: 0,
            long_press: LongPressState::default(),
            enter_behavior: EnterBehavior::default(),
            confirm_requested: false,
        }
    }

    /// Builder: set the Enter-key behavior on this state.
    #[must_use]
    pub fn with_enter(mut self, behavior: EnterBehavior) -> Self {
        self.enter_behavior = behavior;
        self
    }

    /// Advance clocks by frame delta. Called once per frame before processing input.
    pub fn tick(&mut self, delta_ms: u32) {
        self.blink_ms = self.blink_ms.wrapping_add(delta_ms);
        self.monotonic_ms = self.monotonic_ms.wrapping_add(delta_ms);
    }

    /// Insert a character at the cursor position and advance cursor.
    pub fn insert_char(&mut self, ch: char) {
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        // Auto-disable shift after typing a letter (unless caps lock).
        // Don't touch shift in Symbols layer — it controls sub-layer toggle.
        if self.active_layer == LayerId::Letters && !self.caps_lock {
            self.shift_active = false;
        }
        self.blink_ms = 0;
    }

    /// Delete the character before the cursor (backspace).
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            // Find the previous char boundary
            // cursor is always on a char boundary (maintained by insert_char)
            #[expect(clippy::string_slice, reason = "cursor tracks char boundaries")]
            let prev = self.text[..self.cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i);
            self.text.drain(prev..self.cursor);
            self.cursor = prev;
            self.blink_ms = 0;
        }
    }

    /// Whether shift should be applied to character output.
    #[must_use]
    pub fn is_shifted(&self) -> bool {
        self.shift_active || self.caps_lock
    }

    /// Caps lock double-tap window (ms).
    const DOUBLE_TAP_MS: u32 = 400;

    /// Handle shift key press.
    ///
    /// Two rapid presses (within [`DOUBLE_TAP_MS`](Self::DOUBLE_TAP_MS)) →
    /// caps lock, regardless of the intermediate on→off transition.
    /// Single press toggles on/off. Caps lock + press → everything off.
    pub fn toggle_shift(&mut self) {
        let now = self.monotonic_ms;

        if self.caps_lock {
            self.caps_lock = false;
            self.shift_active = false;
            self.last_shift_ms = None;
            return;
        }

        // Check for double-tap → caps lock (two presses within the window)
        if let Some(last) = self.last_shift_ms
            && now.wrapping_sub(last) < Self::DOUBLE_TAP_MS
        {
            self.caps_lock = true;
            self.shift_active = false;
            self.last_shift_ms = None;
            return;
        }

        // Single toggle
        self.shift_active = !self.shift_active;
        self.last_shift_ms = Some(now);
    }
}

/// Result of a keyboard render frame.
#[derive(Debug)]
pub enum KeyboardResult {
    /// Still editing — no action taken.
    Editing,
    /// User confirmed input with the current text.
    Confirmed(String),
    /// User cancelled input.
    Cancelled,
}

#[cfg(test)]
mod tests {
    //! Pure-state unit tests for `KeyboardState`.
    //!
    //! Long-press transitions live in `render::update_long_press` and are
    //! interleaved with `InteractionState` / audio sinks; they're exercised
    //! by the gallery's keyboard scene rather than unit-tested here.

    use super::*;

    fn fresh() -> KeyboardState {
        KeyboardState::new("", "Title", "Placeholder")
    }

    // ── insert_char ────────────────────────────────────────────────────

    #[test]
    fn insert_char_appends_ascii_and_advances_cursor() {
        let mut s = fresh();
        s.insert_char('a');
        s.insert_char('b');
        s.insert_char('c');
        assert_eq!(s.text, "abc");
        assert_eq!(s.cursor, 3);
    }

    #[test]
    fn insert_char_multi_byte_utf8_advances_by_byte_len() {
        let mut s = fresh();
        s.insert_char('é'); // 2 bytes in UTF-8
        assert_eq!(s.text, "é");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn insert_char_emoji_advances_by_full_byte_len() {
        let mut s = fresh();
        s.insert_char('🎹'); // 4 bytes in UTF-8
        assert_eq!(s.text, "🎹");
        assert_eq!(s.cursor, 4);
    }

    #[test]
    fn insert_char_disables_shift_in_letters_layer() {
        let mut s = fresh();
        s.shift_active = true;
        s.active_layer = LayerId::Letters;
        s.insert_char('A');
        assert!(
            !s.shift_active,
            "shift should auto-disable after a letter tap"
        );
    }

    #[test]
    fn insert_char_keeps_shift_in_symbols_layer() {
        let mut s = fresh();
        s.shift_active = true;
        s.active_layer = LayerId::Symbols;
        s.insert_char('!');
        assert!(
            s.shift_active,
            "Symbols layer uses shift for sub-layer toggle"
        );
    }

    #[test]
    fn insert_char_keeps_shift_when_caps_lock() {
        let mut s = fresh();
        s.shift_active = true;
        s.caps_lock = true;
        s.active_layer = LayerId::Letters;
        s.insert_char('A');
        assert!(s.shift_active, "caps lock holds shift across letter taps");
    }

    // ── backspace ──────────────────────────────────────────────────────

    #[test]
    fn backspace_removes_last_ascii_char() {
        let mut s = fresh();
        s.insert_char('a');
        s.insert_char('b');
        s.backspace();
        assert_eq!(s.text, "a");
        assert_eq!(s.cursor, 1);
    }

    #[test]
    fn backspace_round_trips_multi_byte_utf8() {
        let mut s = fresh();
        s.insert_char('é');
        s.backspace();
        assert_eq!(s.text, "");
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn backspace_at_cursor_zero_is_noop() {
        let mut s = fresh();
        s.backspace();
        assert_eq!(s.text, "");
        assert_eq!(s.cursor, 0);
    }

    // ── shift state machine ────────────────────────────────────────────

    #[test]
    fn toggle_shift_first_tap_activates_shift() {
        let mut s = fresh();
        s.toggle_shift();
        assert!(s.shift_active);
        assert!(!s.caps_lock);
    }

    #[test]
    fn toggle_shift_double_tap_within_window_engages_caps_lock() {
        let mut s = fresh();
        s.toggle_shift();
        s.tick(KeyboardState::DOUBLE_TAP_MS - 1);
        s.toggle_shift();
        assert!(
            s.caps_lock,
            "second tap inside the window must engage caps lock"
        );
        assert!(!s.shift_active);
    }

    #[test]
    fn toggle_shift_slow_second_tap_does_not_caps_lock() {
        let mut s = fresh();
        s.toggle_shift();
        s.tick(KeyboardState::DOUBLE_TAP_MS); // exactly at the boundary — counts as out of window
        s.toggle_shift();
        assert!(!s.caps_lock);
        assert!(!s.shift_active, "shift toggles off on slow second tap");
    }

    #[test]
    fn toggle_shift_with_caps_lock_clears_everything() {
        let mut s = fresh();
        s.caps_lock = true;
        s.toggle_shift();
        assert!(!s.caps_lock);
        assert!(!s.shift_active);
        assert_eq!(s.last_shift_ms, None);
    }

    // ── is_shifted ────────────────────────────────────────────────────

    #[test]
    fn is_shifted_reflects_either_shift_or_caps_lock() {
        let mut s = fresh();
        assert!(!s.is_shifted());
        s.shift_active = true;
        assert!(s.is_shifted());
        s.shift_active = false;
        s.caps_lock = true;
        assert!(s.is_shifted());
        s.shift_active = true;
        assert!(s.is_shifted());
    }

    // ── tick + clock wrap ─────────────────────────────────────────────

    #[test]
    fn tick_advances_both_clocks() {
        let mut s = fresh();
        s.tick(50);
        assert_eq!(s.blink_ms, 50);
        assert_eq!(s.monotonic_ms, 50);
    }

    #[test]
    fn double_tap_detection_works_across_u32_wrap() {
        // Park monotonic_ms near the u32 wrap boundary so the second
        // tap straddles the rollover and `wrapping_sub` is exercised.
        let mut s = fresh();
        s.monotonic_ms = u32::MAX - 100;
        s.toggle_shift(); // first tap
        s.tick(200); // wraps past zero to monotonic_ms ≈ 99
        s.toggle_shift(); // second tap
        assert!(
            s.caps_lock,
            "elapsed across wrap must register as a small positive interval"
        );
    }

    // ── builder ───────────────────────────────────────────────────────

    #[test]
    fn with_enter_sets_behavior() {
        let s = KeyboardState::new("", "T", "P").with_enter(EnterBehavior::Confirm);
        assert_eq!(s.enter_behavior, EnterBehavior::Confirm);
    }
}
