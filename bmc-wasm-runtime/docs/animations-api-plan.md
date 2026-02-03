# Animation API Plan

> **Status:** Implemented

## Goals

- Ergonomic animation primitives for immediate-mode UI
- CSS-like timing functions (predefined + custom cubic-bezier)
- Support for: interaction feedback, chart animations, rotation transforms
- Minimal overhead (embedded target, no_std compatible)

## Approach: Wrap `keyframe` Crate

Use the [keyframe](https://github.com/hannesmann/keyframe) crate as foundation:
- no_std compatible (`default-features = false`)
- CSS cubic-bezier for custom timing functions
- `#[derive(CanTween)]` for custom types

## SDK API Design

### Re-exports from keyframe

```rust
pub use keyframe::{ease, CanTween, EasingFunction};
```

### Tween Struct (static easing type)

Use when easing function is known at compile time:

```rust
pub struct Tween<T: CanTween, E: EasingFunction = Linear> {
    from: T,
    to: T,
    duration_ms: u32,
    delay_ms: u32,
    elapsed_ms: u32,
    easing: E,
}

impl<T: CanTween + Copy> Tween<T, Linear> {
    pub fn new(from: T, to: T, duration_ms: u32) -> Self;
}

impl<T: CanTween + Copy, E: EasingFunction> Tween<T, E> {
    pub fn with_ease<E2: EasingFunction>(self, easing: E2) -> Tween<T, E2>;
    pub fn delay(self, delay_ms: u32) -> Self;
    pub fn tick(&mut self, delta_ms: u32);
    pub fn value(&self) -> T;
    pub fn progress(&self) -> f32;
    pub fn is_finished(&self) -> bool;
    pub fn reset(&mut self);
    pub fn retarget(&mut self, new_to: T);
}
```

### DynTween Struct (dynamic easing)

Use when easing needs to change at runtime, or for storage in collections:

```rust
pub struct DynTween<T: CanTween> {
    from: T,
    to: T,
    duration_ms: u32,
    delay_ms: u32,
    elapsed_ms: u32,
    easing: fn(f64) -> f64,
}

impl<T: CanTween + Copy> DynTween<T> {
    pub fn new(from: T, to: T, duration_ms: u32, easing: fn(f64) -> f64) -> Self;
    pub fn linear(from: T, to: T, duration_ms: u32) -> Self;
    // Same methods as Tween: delay, tick, value, progress, is_finished, reset, retarget
}
```

### Predefined Easing

Struct types for `Tween::with_ease()`:
```rust
easing::Linear, easing::EaseIn, easing::EaseOut, easing::EaseInOut
easing::EaseInCubic, easing::EaseOutCubic, easing::EaseInOutCubic
// etc.
```

Function pointers for `DynTween::new()`:
```rust
easing::linear, easing::ease_in, easing::ease_out, easing::ease_in_out
easing::ease_in_cubic, easing::ease_out_cubic, easing::ease_in_out_cubic
// etc.
```

### Transform Helpers

For rotation and other transforms:

```rust
pub struct Transform {
    pub translate: (f32, f32),
    pub rotate: f32,           // radians
    pub scale: (f32, f32),
    pub origin: (f32, f32),    // transform origin (center point)
}

impl Transform {
    pub fn identity() -> Self;
    pub fn rotate_around(center: (f32, f32), angle: f32) -> Self;
    pub fn scale_around(center: (f32, f32), scale: (f32, f32)) -> Self;
    pub fn apply_point(&self, x: f32, y: f32) -> (f32, f32);
    pub fn with_translate(self, x: f32, y: f32) -> Self;
    pub fn with_rotate(self, angle: f32) -> Self;
    pub fn with_scale(self, sx: f32, sy: f32) -> Self;
    pub fn with_origin(self, x: f32, y: f32) -> Self;
}

pub fn deg_to_rad(degrees: f32) -> f32;
pub fn rad_to_deg(radians: f32) -> f32;
```

## Usage Patterns

### Simple Fade Animation (static easing)

```rust
use bmc_wasm_sdk::animation::{Tween, easing};

// Fixed easing type, most efficient
static FADE: Tween<f32, easing::EaseOutCubic> = ...;

fn init() {
    FADE = Tween::new(0.0, 1.0, 300).with_ease(easing::EaseOutCubic);
}
```

### Dynamic Easing Changes

```rust
use bmc_wasm_sdk::animation::{DynTween, easing};
use std::cell::RefCell;

thread_local! {
    static PULSE: RefCell<DynTween<f32>> = RefCell::new(DynTween::linear(0.6, 1.0, 1000));
}

fn render(delta_ms: u32) {
    PULSE.with(|t| {
        t.borrow_mut().tick(delta_ms);
        if t.borrow().is_finished() {
            // Can switch easing at runtime
            *t.borrow_mut() = DynTween::new(1.0, 0.6, 1000, easing::ease_in_out);
        }
    });
}
```

### Rotation (Clock Hand)

```rust
let transform = Transform::rotate_around((center_x, center_y), angle);
let (end_x, end_y) = transform.apply_point(center_x, center_y - hand_length);
```

### Button Press Feedback

```rust
if clicked {
    scale_tween = DynTween::new(0.9, 1.0, 150, easing::ease_out_cubic);
}
```

## Module Structure

```
bmc-wasm-sdk/src/
  animation/
    mod.rs          // re-exports + easing function wrappers
    tween.rs        // Tween and DynTween structs
    transform.rs    // Transform helpers
  lib.rs            // pub mod animation;
```

## Dependencies

```toml
[dependencies]
keyframe = { version = "1.1", default-features = false }
```

## Implementation Notes

- `Tween<T, E>` embeds the easing type, enabling compiler optimizations
- `DynTween<T>` uses function pointers for runtime flexibility
- Transform uses Taylor series approximation for sin/cos (no_std compatible)
- All animations are driven by explicit `tick(delta_ms)` calls

## Demo Widget

The `hello-widget` example demonstrates:
- Fade-in header animation
- Pulsing element with ping-pong easing
- Rotating indicator using Transform
- Button scale feedback on click
- Animated counter display
