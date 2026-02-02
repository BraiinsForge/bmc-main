# WASM Widget Runtime

WebAssembly runtime for remote widget overlays on Braiins Deck.

## Architecture

### Target Hardware

- STM32MP157C (Cortex-A7 dual-core @ 650MHz)
- 256MB DDR3 RAM
- 1280×480 touchscreen display

### Host-Side (Rust)

- **wasmi** - Pure Rust WASM interpreter with fuel metering
- **tiny-skia** - 2D software rendering
- **cosmic-text** - Text shaping/layout with i18n support

### WASM-Side

- Raw FFI layer (`host_*` functions)
- Ergonomic Rust SDK (`bmc-wasm-sdk`)

## Widget API

### Exports (widget implements)

```rust
fn init(width: u32, height: u32);  // called once on load
fn render(delta_ms: u32);          // called each frame
```

### Host Functions (widget calls)

```rust
// Drawing
fn host_fill_rect(x: i32, y: i32, w: u32, h: u32, color: u32);
fn host_draw_rounded_rect(x: i32, y: i32, w: u32, h: u32, radius: u32, color: u32);
fn host_draw_text(text_ptr: u32, text_len: u32, x: i32, y: i32, size: u32, color: u32);

// Interaction
fn host_button(key_ptr: u32, key_len: u32, label_ptr: u32, label_len: u32,
               x: i32, y: i32, w: u32, h: u32, style: u32) -> i32;

// Frame control
fn host_request_frame();              // request next frame ASAP
fn host_request_frame_after(ms: u32); // request frame after delay
```

## Immediate-Mode UI

Inspired by Dear ImGui. Buttons return `true` on the frame they're clicked:

```rust
if button("save-btn", x, y, w, h) {
    // clicked this frame
}
```

The host tracks hit regions, touch state, and click detection.

## Frame Control

Widgets control their own frame rate:

- `request_frame()` - continuous animation
- `request_frame_after(ms)` - periodic updates
- No call = paused until touch event

Host auto-clears overlay before `render()` and auto-commits after.

## SDK Usage

```rust
use bmc_wasm_sdk::*;

#[no_mangle]
pub extern "C" fn render(_delta_ms: u32) {
    ui::render(WIDTH, HEIGHT, col(props!(), [
        text("Hello from WASM!", 24, props!()),
        row(props!(gap: 16.0), [
            button(ButtonStyle::Primary, "Click me"),
            button(ButtonStyle::Secondary, "Or me"),
        ]),
    ]));
    request_frame();
}
```

## Development Testbed

```bash
cargo run --bin testbed -- path/to/widget.wasm
```

Features:

- Hot reload on file change
- Mouse → touch translation
- FPS counter

## Resource Limits

- Fuel per frame: 10,000,000 instructions
- Overlay size: matches display (1280×480)
