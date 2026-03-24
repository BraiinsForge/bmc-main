# Declarative Animations for WASM Widgets

## Problem with Current Approach

The current SDK animation system (`sdk/src/animation/`) uses the `keyframe` crate to compute easing values inside WASM:

```rust
// Current: imperative, WASM computes values
animated!(ROTATION: f32);

fn init() {
    ROTATION::start(0.0, PI * 2.0, 4000, easing::linear);
}

fn render(delta_ms: u32) {
    ROTATION::tick(delta_ms);  // WASM does math
    let angle = ROTATION::get();

    // Use value in tree
    rotated(angle, centered(rect(...)))
}
```

### Issues:

- `keyframe`, `mint`, `num-traits`, `libm` compiled into WASM (~57KB binary)
- WASM must manually tick animations
- WASM must handle loop/ping-pong logic
- Conceptual overhead of "managing animations"

## Better Approach: Declarative Node Modifiers

Animations should be declarative properties on nodes, not imperative state in WASM:

```rust
// Proposed: declarative, host computes values
fn render(delta_ms: u32) {
    let tree = col(props!(), [
        canvas(props!(width: 64.0, height: 64.0), [
            rect(0.0, 0.0, 32.0, 32.0, VIOLET_50)
                .animate(Transform::Rotate, 0.0..TAU, 4000, Easing::Linear, Loop::Forever)
                .animate(Transform::Scale, 0.5..1.0, 1000, Easing::EaseInOut, Loop::PingPong)
        ])
    ]);

    render_ui(w, h, tree);
}
```

WASM declares intent. Host:

- Tracks animation state per node
- Computes current values each frame
- Applies transforms during rendering

## Analysis of hello-widget Usage

| Animation | Current Usage                            | Declarative Equivalent                                 |
|-----------|------------------------------------------|--------------------------------------------------------|
| FADE      | `color!(GRAY_10, alpha: fade)`           | `.animate(Alpha, 0.0..1.0, 600, EaseOutCubic, Once)`   | 
| ROTATION  | `rotated(rotation, ...)`                 | `.animate(Rotate, 0.0..TAU, 4000, Linear, Forever)`    | 
| PULSE     | `rect(..., pulse_size, pulse_size, ...)` | `.animate(Scale, 0.5..1.0, 1000, EaseInOut, PingPong)` |

**None of these require WASM to see the actual values.**

## Serialization Format

Animation modifiers serialize as part of the node:

```
[NODE_TYPE]
[node data...]
[animation_count: u8]
[animations...]

Animation:
[property: u8]      // Rotate, Scale, Alpha, TranslateX, TranslateY
[from: f32]
[to: f32]
[duration_ms: u16]
[easing: u8]        // Linear, EaseIn, EaseOut, EaseInOut, ...
[loop_mode: u8]     // Once, Forever, PingPong
```

## Host State

```rust
struct AnimationState {
    property: AnimationProperty,
    from: f32,
    to: f32,
    duration_ms: u16,
    easing: Easing,
    loop_mode: LoopMode,
    elapsed_ms: u32,
    direction: bool,  // For ping-pong
}

// Keyed by node identity (need stable node IDs or position-based)
// animations: HashMap<NodeKey, Vec<AnimationState>>
```

## Benefits

| Aspect              | Current (Imperative)            | Proposed (Declarative) |
|---------------------|---------------------------------|------------------------|
| Binary size         | ~57KB                           | ~20KB (estimate)       |
| SDK complexity      | 668 lines + keyframe            | ~100 lines             | 
| WASM responsibility | Tick, query, loop logic         | Declare intent         |
| Host responsibility | None                            | Track & apply          | 
| Dependencies        | keyframe,mint, num-traits, libm | None                   |

## Edge Cases

**Derived values:** `let size = 32.0 * pulse`

- Solution: Animate the actual property (`width: 16..32`) instead of a multiplier

**Conditional animations:** Start animation on button click

- Solution: Animation `enabled: bool` flag, or trigger via node presence

**Chained animations:** A then B

- Solution: `delay_ms` field, or `Loop::Once` with sequencing

**Dynamic targets:** Animate to user-selected position

- Solution: Allow `to` to reference state, or re-declare animation with new target

## Implementation Priority

1. **Current:** Ship modal API with host-side animation (already implemented)
2. **Next:** Design declarative animation API for draw commands
3. **Later:** Deprecate `sdk/src/animation/` module
4. **Eventually:** Remove keyframe dependency

## Open Questions

- Node identity for animation state tracking (stable IDs vs tree position)?
- How to handle animations on dynamically added/removed nodes?
- Should animations auto-start or require explicit trigger?
