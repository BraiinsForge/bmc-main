# Pomodoro Widget + LED Host API — Implementation Plan

## Context

BDK-290 calls for exposing the LED peripheral to WASM widgets and building a Pomodoro timer as the driving use case. The
SDK has audio (BDK-359) but zero LED access. This plan adds the LED API following the proven audio pattern, then builds
the pomodoro widget to exercise it.

## Stage 1: Extract LED Data Types to `bmc-shared/led-data` — Complete

Extracted `Rgb`, `LedEffect`, `LedScene`, `LedCommand`, `LedEvent` into `bmc-shared/led-data`. `bmc-led/src/data.rs`
re-exports via `pub use bmc_shared_led_data::*`.

---

## Stage 2: LED Host API — Complete

SDK (`sdk/src/led.rs`): `LedEffect` enum, FFI bindings, safe wrappers (`set_effect`, `stop`).

Host (`runtime_wasmi.rs`): 3 linker bindings forwarding to `HostState.led_request_sender`.

---

## Stage 3: Pomodoro Widget — Core — Complete

Widget at `widgets-wasm-examples/pomodoro/` with:

- Phase state machine (Idle → Working → ShortBreak → Working → ... → LongBreak → Idle)
- LED mapping per phase (Breathe red, Solid green/blue, Chase on cycle complete)
- Audio chimes on transitions
- Multi-size UI (Small, Medium, Large, Full) with countdown, session dots, transport buttons
- Settings modal with NumberInput components for work/short/long durations

---

## Stage 4: KV Persistence + Config UI — Complete

- KV persistence for durations and daily total (`pomodoro_work_min`, etc.)
- Daily reset via stored date string
- Settings modal using `NumberInputProps` + `ModalFooter` (Save button)
- `number_input_handle()` for +/- click handling with clamping

### SDK components added

- **NumberInput** (`sdk/src/number_input.rs`): `NumberInputProps`, `number_input!` macro, `number_input_handle()`.
  CDS-style with label, suffix, warning/error states, built-in stepper icons (`ICON_MINUS`, `ICON_PLUS`,
  `ICON_WARN_ALT`, `ICON_WARN_FILLED`).
- **Modal footer** (`ModalFooter` on `ModalProps`): declarative `ModalAction` primary/secondary buttons with `danger`
  flag. Host renders buttons at adaptive size (compact on small viewports). CDS layout: `[secondary | primary]` or
  `[spacer | primary]`.
- **Modal API**: plain `modal()` function (replaced `modal()`/`modal_styled()` and aborted `modal!` macro). `ModalProps`
  includes `height`, `footer`, `margin`, colors, `max_width`.
- **Modal body scroll**: reuses `TreeNode::Scroll` instead of custom scroll code.
- **Compact modal**: host auto-adapts header (32px), body padding (8px), button size (Small) for viewports ≤ 300px.
- **`SizeVariant::width()`/`height()`**: canonical dimension accessors.

---

## Stage 5: Fixtures + Visual Regression — Not Started

- Create `widgets-wasm-examples/pomodoro/config.toml` for capture configuration
- Record fixtures, generate baseline screenshots
- Verify via `make regression-test EXAMPLE=pomodoro`

---

## Verification

1. `make validate-wasm` — all examples build, clippy clean, tests pass
2. Run testbed with pomodoro → timer works, LED effects, audio plays, settings modal functional
3. All modal callers (hello-widget, media-control, pomodoro) migrated to `modal()` function
