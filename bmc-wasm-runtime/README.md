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
│  - Anim decl    │     │  - Taffy layout │
│  - State        │     │  - Render       │
└─────────────────┘     └─────────────────┘
```

Widgets build a declarative UI tree that gets serialized and sent to the host for layout and rendering. Animations and
transitions are declared in the tree and computed host-side, keeping WASM binaries small by offloading text shaping,
layout, and animation math to native code.

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
- `modal(id, is_open, title, content_height, body)` - Modal dialog overlay
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

### Macros

```rust
// Layout props for containers
props!(padding: 16.0, gap: 8.0, background: GRAY_90)

// Text + layout props combined
style!(size: 20, weight: 700, color: VIOLET_50, padding: 8.0)

// Lightweight format!() replacement (~5KB smaller WASM binary)
fmt!("Count: {}", value)
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

### Modal Dialogs

```rust
modal(1, is_open, "Settings", content_height, [
    text("Modal body content", style!(size: 14)),
    button(ButtonStyle::Primary, "Save"),
])
```

Host manages backdrop animation (fade), scroll state (drag + wheel), and renders a close button in the header. The close
button is the first button inside the modal — handle it via `TreeRenderResult::clicks`.

```rust
let result = render_ui(width, height, root);

// Button 0 in the modal is the close button
if result.clicks.get(0) == Some(&true) {
    is_open = false;
}
```

Use `modal_styled` with `ModalProps` for custom padding or backdrop alpha.

### Animations

Animations are declared on draw commands and computed host-side. No animation crate needed in WASM.

#### `.animate()` — Repeating host-driven effects

```rust
use AnimProperty::*;
use Easing::*;
use LoopMode::*;

// Spin forever + pulse scale
rect(0.0, 0.0, 32.0, 32.0, VIOLET_50)
    .animate(Rotate, 0.0, TAU, 4_000, Linear, Forever)
    .animate(Scale, 0.5, 1.0, 1_000, EaseInOut, PingPong)

// Orbiting element
orbit(18.0, 0.0, rect(0.0, 0.0, 4.0, 4.0, VIOLET_40))
    .animate(OrbitAngle, -FRAC_PI_2, 3.0 * FRAC_PI_2, 4_000, Linear, Forever)

// Color animation
rect(0.0, 0.0, 32.0, 32.0, RED_50)
    .animate_color(RED_50, VIOLET_50, 2_000, EaseInOut, PingPong)
```

Properties: `Rotate`, `Scale`, `Alpha`, `TranslateX`, `TranslateY`, `OrbitAngle`, `Color`

#### `.transition()` — Smooth state-driven interpolation

Like CSS `transition`. WASM sets target values each render; host smoothly interpolates when values change.

```rust
// Smooth color change when state changes
rect(0.0, 0.0, 32.0, 32.0, current_color)
    .transition(300, EaseOutCubic)

// Custom color space for transitions
rect(0.0, 0.0, 32.0, 32.0, current_color)
    .transition_with_color_space(300, EaseOutCubic, Oklch)
```

Host auto-requests frames when any animation or transition is active — no manual `request_frame()` needed.

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
