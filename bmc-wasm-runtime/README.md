# WASM Widget Runtime

WebAssembly runtime for Braiins Deck remote widget overlays. Widgets are compiled to WASM and rendered with host-side
flex layout (Taffy) and text shaping (cosmic-text).

## Architecture

```
┌─────────────────┐     ┌─────────────────┐
│   WASM Widget   │     │      Host       │
│  (bmc-wasm-sdk) │────▶│ (bmc-wasm-runtime)
│                 │     │                 │
│  - UI tree      │     │  - Deserialize  │
│  - Animations   │     │  - Taffy layout │
│  - State        │     │  - Render       │
└─────────────────┘     └─────────────────┘
```

Widgets build a declarative UI tree that gets serialized and sent to the host for layout and rendering. This keeps WASM
binaries small (~15KB) by offloading text shaping and layout to native code.

## SDK API

### Layout Primitives

```rust
use bmc_wasm_sdk::*;

render_ui(width, height,
    col(props!(padding: 24.0, gap: 16.0), [
        row(props!(gap: 12.0), [
            button(ButtonStyle::Primary, "Click me"),
            button(ButtonStyle::Secondary, "Cancel"),
        ]),
        spacer(1.0),
    ])
);
```

- `col(props, children)` - Vertical flex container
- `row(props, children)` - Horizontal flex container
- `center(props, children)` - Centered container
- `spacer(flex)` - Flexible spacer
- `button(style, label)` - Interactive button
- `canvas(props, draws)` - Custom drawing area

### Text

```rust
// Simple text
text("Hello world", style!(size: 24, color: GRAY_10))

// Rich paragraph with styled spans
paragraph(style!(size: 14, line_height: 1.4), [
    span("Click ", ()),
    span("Save", style!(weight: 700)),
    span(" to ", ()),
    span("confirm", style!(color: GREEN_50)),
    span(".", ()),
])
```

Text wraps automatically to container width. Use `max_width` in style to set an explicit maximum.

### Style Macros

```rust
// Layout props for containers
props!(padding: 16.0, gap: 8.0, background: GRAY_90)

// Text + layout props combined
style!(size: 20, weight: 700, color: VIOLET_50, padding: 8.0)
```

**Text style fields:** `size`, `weight`, `italic`, `underline`, `strikethrough`, `line_height`, `align`, `max_width`

**Layout fields:** `padding`, `margin`, `gap`, `flex`, `width`, `height`, `background`

**Shared:** `color` (applies to both text and layout)

### Canvas Drawing

```rust
canvas(props!(width: 100.0, height: 100.0), [
    rect(10.0, 10.0, 20.0, 20.0, RED_50),
    centered(rect(0.0, 0.0, 32.0, 32.0, VIOLET_50)),
    orbit(40.0, angle, rect(0.0, 0.0, 8.0, 8.0, GREEN_50)),
    rotated(rotation, rect(0.0, 0.0, 16.0, 16.0, GRAY_10)),
])
```

### Animations

```rust
animated!(FADE: f32);
animated!(ROTATION: f32);

fn init() {
    FADE::start(0.0, 1.0, 500, easing::ease_out_cubic);
    ROTATION::start(0.0, PI * 2.0, 2000, easing::linear);
}

fn render(delta_ms: u32) {
    FADE::tick(delta_ms);
    ROTATION::tick(delta_ms);

    let opacity = FADE::get();
    let angle = ROTATION::get();

    if ROTATION::is_finished() {
        ROTATION::reset(); // Loop
    }
}
```

### Colors

Brand colors from the design system:

```rust
GRAY_10..GRAY_100    // Light to dark grays
VIOLET_10..VIOLET_100 // Brand purple
GREEN_10..GREEN_100   // Success
RED_10..RED_100       // Error/danger
ORANGE_10..ORANGE_100 // Warning
TRANSPARENT

// With alpha
color!(GRAY_80, alpha: 0.5)
```

## Development

```bash
# Run testbed with hot reload
make dev

# Build and run release
make run

# Check WASM binary size
make size
```

## Crate Structure

- `bmc-wasm-runtime` - Host runtime (layout, rendering)
- `bmc-wasm-sdk` - Widget SDK (compiled to WASM)
- `bmc-wasm-protocol` - Shared types and constants
