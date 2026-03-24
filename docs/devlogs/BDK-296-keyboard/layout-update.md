# On-screen Keyboard: Layout Update from AnySoftKeyboard

**Ticket:** BDK-296 **Date:** 2026-03-25

## What changed

All 6 keyboard layout XMLs updated to the latest from the AnySoftKeyboard monorepo (moved from the archived
`LanguagePack` repo to `AnySoftKeyboard/AnySoftKeyboard/addons/languages/`).

## popupKeyboard resolution

The new upstream layouts use `android:popupKeyboard="@xml/de_popup_e"` references instead of inline
`android:popupCharacters` for some keys. This references a separate XML file containing a mini keyboard layout for the
popup.

`build.rs` now resolves these at compile time:

1. Detects `popupKeyboard` attribute when `popupCharacters` is absent.
2. Loads the referenced XML from the layouts directory.
3. Extracts character codes from its `<Key>` elements.
4. Skips control characters (< 32) — the Norwegian popup keyboards have an upstream bug where digit values (3, 6, 7, 8,
   9\) are used instead of ASCII codepoints (51, 54, 55, 56, 57).
5. Skips ASCII digits to avoid duplication with `normalize_top_row_popups`.

10 popup keyboard XMLs were added (English: 2, German: 1, Norwegian: 7).

## Superscript/subscript deduplication

The French layout's upstream popupCharacters include `¹₁` alongside the plain digit `1`. `dedup_digit_variants()` strips
superscript (⁰¹²³⁴⁵⁶⁷⁸⁹) and subscript (₀₁₂₃₄₅₆₇₈₉) variants when the corresponding ASCII digit is already present.

## Fallback font

NotoSans-Regular.ttf (OFL licensed, 500KB) added as a fallback font. Greek characters (ε, η, ρ, ϕ, κ, etc.) in the
English layout's popup data previously rendered as TOFU because BraiinsSans doesn't cover them.

The fallback is registered with both FemtoVG and cosmic-text, but cosmic-text paragraphs explicitly request
`Family::Name("Braiins Sans")` to prevent metric changes for existing Latin text. See `keyboard-skinning.md` for details
on the regression this caused and how it was resolved.
