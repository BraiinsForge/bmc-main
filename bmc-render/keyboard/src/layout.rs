// Copyright (C) 2026  Braiins Systems s.r.o.

//! Keyboard layout definitions.
//!
//! Letter rows are generated from AnySoftKeyboard XML (Apache 2.0).
//! Bottom row and symbols layers are shared across all layouts and defined here.
//! See: <https://github.com/AnySoftKeyboard/AnySoftKeyboard/tree/main/addons/languages>

/// A complete keyboard layout with multiple layers.
#[derive(Debug)]
pub struct KeyboardLayout {
    /// Language / region name (e.g. "US English", "Czech").
    pub name: &'static str,
    /// Layout variant (e.g. "QWERTY", "QWERTZ", "AZERTY").
    pub variant: &'static str,
    /// Rows of keys for the letters layer (base). Includes bottom row.
    pub letters: &'static [&'static [Key]],
    /// Rows of keys for the letters layer (shifted). Includes bottom row.
    pub letters_shifted: &'static [&'static [Key]],
    /// Rows of keys for the symbols layer.
    pub symbols: &'static [&'static [Key]],
    /// Rows of keys for the symbols shifted layer (more symbols).
    pub symbols_shifted: &'static [&'static [Key]],
}

/// A single key definition.
#[derive(Debug, Clone, Copy)]
pub struct Key {
    /// Display label on the key.
    pub label: &'static str,
    /// Long-press popup characters. First char is shown as hint in top-right corner.
    /// Empty string if no popups available.
    pub popup: &'static str,
    /// Width in relative units (1.0 = standard key width).
    pub width: f32,
    /// What the key does when pressed.
    pub code: KeyCode,
}

/// What a key does when pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    /// Emit a character.
    Char(char),
    /// Backspace / delete.
    Backspace,
    /// Enter / return (confirms input in our case).
    Enter,
    /// Space bar.
    Space,
    /// Toggle shift / caps lock.
    Shift,
    /// Switch to a different layer.
    SwitchLayer(LayerId),
    /// Toggle between sub-layers within the current layer (e.g. symbols ↔ symbols-shifted).
    ToggleSubLayer,
}

/// Which layer of the keyboard is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerId {
    Letters,
    Symbols,
}

impl Key {
    pub(crate) const fn char(label: &'static str, ch: char) -> Self {
        Self {
            label,
            popup: "",
            width: 1.0,
            code: KeyCode::Char(ch),
        }
    }

    pub(crate) const fn char_popup(label: &'static str, popup: &'static str, ch: char) -> Self {
        Self {
            label,
            popup,
            width: 1.0,
            code: KeyCode::Char(ch),
        }
    }

    pub(crate) const fn char_wide(label: &'static str, ch: char, width: f32) -> Self {
        Self {
            label,
            popup: "",
            width,
            code: KeyCode::Char(ch),
        }
    }

    pub(crate) const fn action(label: &'static str, width: f32, code: KeyCode) -> Self {
        Self {
            label,
            popup: "",
            width,
            code,
        }
    }
}

impl KeyboardLayout {
    /// Get the active rows for the current layer and shift state.
    #[must_use]
    pub fn active_rows(&self, layer: LayerId, shifted: bool) -> &[&[Key]] {
        match (layer, shifted) {
            (LayerId::Letters, false) => self.letters,
            (LayerId::Letters, true) => self.letters_shifted,
            (LayerId::Symbols, false) => self.symbols,
            (LayerId::Symbols, true) => self.symbols_shifted,
        }
    }
}

// ── Shared bottom row (appended to all letter layouts) ──────────────

static BOTTOM_ROW: &[Key] = &[
    Key::action("?123", 1.5, KeyCode::SwitchLayer(LayerId::Symbols)),
    Key::char_wide(",", ',', 1.0),
    Key::action(" ", 5.0, KeyCode::Space),
    Key::char_wide(".", '.', 1.0),
    Key::action("", 1.5, KeyCode::Enter),
];

static BOTTOM_ROW_SYMBOLS: &[Key] = &[
    Key::action("ABC", 1.5, KeyCode::SwitchLayer(LayerId::Letters)),
    Key::char_wide(",", ',', 1.0),
    Key::action(" ", 5.0, KeyCode::Space),
    Key::char_wide(".", '.', 1.0),
    Key::action("", 1.5, KeyCode::Enter),
];

// ── Shared symbols layers ───────────────────────────────────────────

static SYMBOLS_ROW1: &[Key] = &[
    Key::char("1", '1'),
    Key::char("2", '2'),
    Key::char("3", '3'),
    Key::char("4", '4'),
    Key::char("5", '5'),
    Key::char("6", '6'),
    Key::char("7", '7'),
    Key::char("8", '8'),
    Key::char("9", '9'),
    Key::char("0", '0'),
];

static SYMBOLS_ROW2: &[Key] = &[
    Key::char("@", '@'),
    Key::char("#", '#'),
    Key::char("$", '$'),
    Key::char("%", '%'),
    Key::char("&", '&'),
    Key::char("-", '-'),
    Key::char("+", '+'),
    Key::char("(", '('),
    Key::char(")", ')'),
];

static SYMBOLS_ROW3: &[Key] = &[
    Key::action("=\\<", 1.5, KeyCode::ToggleSubLayer),
    Key::char("*", '*'),
    Key::char("\"", '"'),
    Key::char("'", '\''),
    Key::char(":", ':'),
    Key::char(";", ';'),
    Key::char("!", '!'),
    Key::char("?", '?'),
    Key::action("", 1.5, KeyCode::Backspace),
];

static SYMBOLS: &[&[Key]] = &[SYMBOLS_ROW1, SYMBOLS_ROW2, SYMBOLS_ROW3, BOTTOM_ROW_SYMBOLS];

static SYMBOLS_SHIFTED_ROW1: &[Key] = &[
    Key::char("~", '~'),
    Key::char("`", '`'),
    Key::char("|", '|'),
    Key::char("\u{2022}", '\u{2022}'), // •
    Key::char("\u{221A}", '\u{221A}'), // √
    Key::char("\u{03C0}", '\u{03C0}'), // π
    Key::char("\u{00F7}", '\u{00F7}'), // ÷
    Key::char("\u{00D7}", '\u{00D7}'), // ×
    Key::char("{", '{'),
    Key::char("}", '}'),
];

static SYMBOLS_SHIFTED_ROW2: &[Key] = &[
    Key::char("\u{00A3}", '\u{00A3}'), // £
    Key::char("\u{00A2}", '\u{00A2}'), // ¢
    Key::char("\u{20AC}", '\u{20AC}'), // €
    Key::char("\u{00A5}", '\u{00A5}'), // ¥
    Key::char("^", '^'),
    Key::char("\u{00B0}", '\u{00B0}'), // °
    Key::char("=", '='),
    Key::char("[", '['),
    Key::char("]", ']'),
];

static SYMBOLS_SHIFTED_ROW3: &[Key] = &[
    Key::action("123", 1.5, KeyCode::ToggleSubLayer),
    Key::char("\\", '\\'),
    Key::char("/", '/'),
    Key::char("_", '_'),
    Key::char("<", '<'),
    Key::char(">", '>'),
    Key::char("\u{00AB}", '\u{00AB}'), // «
    Key::char("\u{00BB}", '\u{00BB}'), // »
    Key::action("", 1.5, KeyCode::Backspace),
];

static SYMBOLS_SHIFTED: &[&[Key]] = &[
    SYMBOLS_SHIFTED_ROW1,
    SYMBOLS_SHIFTED_ROW2,
    SYMBOLS_SHIFTED_ROW3,
    BOTTOM_ROW_SYMBOLS,
];

// ── Generated layouts from AnySoftKeyboard XML ──────────────────────

include!(concat!(env!("OUT_DIR"), "/layouts_generated.rs"));
