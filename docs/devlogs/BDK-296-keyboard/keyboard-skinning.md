# On-screen Keyboard: Skinning & Theme System

**Ticket:** BDK-296 **Date:** 2026-03-25

## What was built

A theming system for the on-screen keyboard that replaces ~20 hardcoded color references with a cascading
`KeyboardTheme` struct. Three built-in themes (Carbon Dark, Carbon Light) plus runtime skin loading from the generic
`Skin` system.

## Key design decisions

### KeyStyle as a tagged union (Flat | NinePatch)

A key is rendered with either solid colors or 9-patch bitmaps — never both. Making this a tagged union prevents
nonsensical states (color bg + 9-patch bg simultaneously). Both variants carry `fg` and `fg_pressed` so the foreground
color always transitions with the background.

The `Flat` variant includes an optional `border` field (TRANSPARENT = none) even though no keys currently use borders —
skins may want them.

### Theme resolution cascade

`resolve_key_style()` checks in order:

1. Per-key overrides (`keys.overrides` array, case-insensitive for `Char` variants)
2. Shift state overrides (caps lock / shift active — full KeyStyle for contrast safety)
3. Group defaults (`keys.alpha` vs `keys.fn_keys` based on KeyCode variant)

`KeyboardTheme` groups properties into nested sub-structs: `InputStyle` (text field), `PopupStyle` (long-press popup),
and `KeyDefaults` (key groups, shift states, overrides). Palette keys follow a predictable naming convention:
`key_group_alpha_*`, `key_group_fn_*` for groups, `shift_active_*` / `shift_lock_*` for shift states, and
`key_value_{ch}_*` for per-key overrides. Inheritance is resolved at construction time: `key_value_*` →
`key_group_alpha_*`, `shift_*` → `key_group_fn_*`, `key_group_fn_*` → `key_group_alpha_*` → `CARBON_DARK`.

Shift state overrides are full `KeyStyle` (not just bg color) because a bright shift-active background may need a
different fg color for contrast. We learned this when the Llama skin's bright 9-patch keys needed dark text — if shift
overrides only changed bg, the bright bg would get the wrong fg.

### Skin system generalization

The `Skin` type was refactored from a fixed-field `SkinPalette` struct to a generic model:

- **`[palette]`** — freeform string→Color map. Each consumer defines its own key names.
- **`[assets.*]`** — image assets with optional `color` field for text/icon color.
- **`Skin::color_or("name", fallback)`** — palette lookup with fallback.
- **`Skin::get_nine_patch("name")`** — asset lookup with runtime bitmap registration.

The old `SkinPalette` had 6 hardcoded fields (background, layer1, layer2, text_primary, text_secondary, accent) that
every consumer was forced to reinterpret. Now each consumer (media control, keyboard) defines its own vocabulary — no
shared palette contract.

### Font fallback without metric regression

Adding NotoSans as a fallback font for Greek/Cyrillic glyphs initially broke snapshot tests across all widgets. The
cause: cosmic-text's `FontSystem` uses `fontdb` for font selection. When paragraphs requested `Family::SansSerif`,
cosmic-text could pick Noto over BraiinsSans for some glyphs, changing text metrics.

The fix: paragraphs now request `Family::Name("Braiins Sans")` explicitly. Cosmic-text always prefers BraiinsSans for
Latin text (identical metrics to before) and only falls back to Noto for missing glyphs. FemtoVG's simple text path
(`draw_text`/`measure_text`) uses an explicit font list: `set_font(&[font_regular, font_fallback])`.

## Skin format (skin.toml)

```toml
name = "Llama"
description = "Classic media player aesthetic"

# Palette keys follow a naming convention:
#   key_group_alpha_*  — character keys (alpha group)
#   key_group_fn_*     — function keys (fn group, inherits from alpha)
#   shift_active_*     — shift key when active (inherits from fn)
#   shift_lock_*       — shift key when caps lock (inherits from fn)
#   key_value_{ch}_*   — per-key override (inherits from alpha)
[palette]
background = "#151520"
key_group_alpha_hint = "#505068"
popup_fg = "#e0e0ee"
popup_selected_bg = "#4a8a4a"

# Image assets — matched to .9.png files by name
[assets.key]
color = "#181828"          # text color on this 9-patch

[assets.key_pressed]
color = "#080810"
```

Each `[assets.<name>]` must have a corresponding `<name>.9.png` or `<name>.png` in the skin directory/zip. The `color`
field is optional — it overrides the palette text color for that specific element.

## Storybook interaction fixes

### Viewport pan suppression

Frame widgets in the storybook use `Sense::click_and_drag()` so egui's `ScrollArea` doesn't steal drag events. Previous
attempts at conditional sensing (only claim drag when `any_touch_down()`) failed due to a 1-frame lag: events are
collected in the egui layout pass but only processed in the next frame's `begin_frame()`. By then the ScrollArea had
already claimed the drag. The fix: always use `click_and_drag` for frame widgets. Scroll still works via mouse wheel and
dragging on non-frame areas.

### Scroll offset in coordinate mapping

Pointer events are mapped from screen coordinates to FBO coordinates using the widget rect origin. When the storybook
viewport was scrolled, the mapping used `visible.min` (the clipped rect) instead of `rect.min` (the full widget rect),
causing key hits to land on the wrong keys. Fixed by passing both rects separately — `fbo_rect` for coordinate mapping,
`visible` for hit testing.
