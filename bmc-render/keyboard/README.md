# bmc-render-keyboard

Modal on-screen keyboard rendered via \[`bmc_render::renderer::Renderer`\]. Tap-based text input with multiple layouts
(QWERTY, QWERTZ, AZERTY, Nordic); layout data derived from AOSP LatinIME (Apache 2.0). When active the keyboard takes
over the entire screen — widgets request text input and receive the completed string, they never see the keyboard UI.

## Usage

```rust
use bmc_render_keyboard::{
    render_keyboard, EnterBehavior, KeyboardCtx, KeyboardResult, KeyboardState,
    KeyboardTheme, SilentSink,
};

let mut state = KeyboardState::new("", "Wi-Fi Password", "")
    .with_enter(EnterBehavior::Confirm);
let theme = KeyboardTheme::default();
let mut audio = SilentSink;

// Per frame:
let ctx = KeyboardCtx {
    renderer: &mut renderer,
    interaction: &mut interaction,
    state: &mut state,
    audio: &mut audio,
    theme: &theme,
    width,
    height,
    delta_ms,
};
match render_keyboard(ctx) {
    KeyboardResult::Editing => {}
    KeyboardResult::Confirmed(text) => { /* commit */ }
    KeyboardResult::Cancelled => { /* dismiss */ }
}
```

## Public API

- `KeyboardState` — persistent session state (text, cursor, layer, shift, clocks).
- `KeyboardCtx` — per-frame bundle: renderer, interaction, state, audio, theme, size, delta.
- `KeyboardResult` — `Editing` / `Confirmed(String)` / `Cancelled`.
- `EnterBehavior` — `Disabled` (default, dimmed inert key) / `InsertNewline` / `Confirm`.
- `KeyboardTheme`, `KeyDefaults`, `KeyOverride`, `KeyStyle`, `InputStyle`, `PopupStyle` — skinning surface.
- `AudioSink` + `SilentSink`, `SoundTag`, `KeySound` — host-provided audio playback.
- `KeyboardLayout`, `LayerId`, `Key`, `KeyCode` — layout types.

## Theming & skinning

Per-key style resolves through a cascade: explicit `KeyOverride` → group default (letter / number / function / action) →
palette lookup. `KeyOverride::Char` matches case-insensitively for ASCII letters only; non-ASCII chars compare exactly.
Skin assets bind through `Renderer::register_icon`/`register_font` by stable \[`SoundTag`\]-style string keys.

See [`docs/devlogs/BDK-296-keyboard/keyboard-skinning.md`](../../docs/devlogs/BDK-296-keyboard/keyboard-skinning.md) for
the design rationale and the font-fallback / theme cascade history.

## Layouts

Layouts are compiled from AOSP LatinIME XML at build time. The pipeline unconditionally surfaces the number row and
dedups the AOSP super/subscript keys that the Deck's small grid can't usefully render. Popup keys (long-press
alternates) are resolved at compile time and shifted into the variants the runtime renders.

See [`docs/devlogs/BDK-296-keyboard/layout-update.md`](../../docs/devlogs/BDK-296-keyboard/layout-update.md) for the XML
pipeline and the popup/dedup decisions.

## Device integration (open work, tracked under BDK-296)

- **D1 — GL context sharing.** Compositor uses Smithay's `GlesRenderer`; the keyboard needs `FemtoVgRenderer`. Plan:
  build a `FemtoVgRenderer` in the compositor thread sharing the same EGL context, render to an offscreen target via
  `begin_frame_to_image()`, composite as a texture in `render_scene()`. Risk: GL state conflicts between FemtoVG and
  Smithay; fallback is DMA-BUF import.
- **D2 — Touch routing.** The keyboard is fully modal, so all touch events route to it while visible; the compositor's
  `SeatHandler` stays stubbed.
- **Stage 3 — Compositor overlay.** `KeyboardOverlay` owns the shared-context `FemtoVgRenderer`; while the overlay is
  visible the compositor skips widget compositing entirely and renders the keyboard texture directly.
- **Stage 4 — Widget text-input API.** `host_request_text_input()` host import + completion plumbing back to widgets.
  Blocked on Stage 3.

## Assumptions

Single-touch hardware (the Deck). `LongPressState` keys transitions off `(row, col)` rather than a touch identifier — on
multi-touch a lift-and-retouch on a different key could abort `Waiting` based on a stale signal. Revisit if the keyboard
ever ships on multi-touch hardware.
