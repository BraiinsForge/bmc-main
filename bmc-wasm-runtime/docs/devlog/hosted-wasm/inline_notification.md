# Inline Notification Component + SpaceX Widget Error Handling

**Status: Completed.** All sections implemented. The code is the source of truth for current details.

## Context

The SpaceX widget silently retries on fetch failure — the user sees "Loading…" forever with no indication of what went
wrong. We need an inline notification component (inspired by Carbon Design System) to display error/warning/info/success
messages, and then wire it into the SpaceX widget for error states.

The component will be a **new node type** in the protocol/SDK/host rendering pipeline, not just a widget-side
composition of existing primitives. This keeps the widget code clean and establishes a reusable pattern for all future
widgets.

---

## 1. Add notification icons as builtins

Carbon provides `error--solid.svg`, `warning--solid.svg`, `checkmark--solid.svg`, `info--solid.svg` in
`/home/kubijo/dev/carbon-icons/src/svg/`. These are 16×16 single-path SVGs.

Copy the 4 SVGs into `bmc-wasm-runtime/icons/` (alongside existing `close.svg`), and register them in `build.rs` with
reserved IDs:

| Icon    | File                   | Builtin ID          |
| ------- | ---------------------- | ------------------- |
| close   | `close.svg`            | `0xFF01` (existing) |
| error   | `error--solid.svg`     | `0xFF10`            |
| warning | `warning--solid.svg`   | `0xFF11`            |
| success | `checkmark--solid.svg` | `0xFF12`            |
| info    | `info--solid.svg`      | `0xFF13`            |

Expose constants in `protocol/src/icon.rs` so both SDK and host can reference them.

**Files:**

- `icons/error--solid.svg`, `warning--solid.svg`, `checkmark--solid.svg`, `info--solid.svg` (copy from carbon-icons)
- `build.rs` — add 4 new icon ID mappings
- `protocol/src/icon.rs` — add `ICON_BUILTIN_ERROR`, `ICON_BUILTIN_WARNING`, etc.

---

## 2. Add `NODE_NOTIFICATION` to the protocol

New node type `0x08` in `protocol/src/nodes.rs`.

**Wire format:**

```
[NODE_NOTIFICATION: u8]
[kind: u8]              // 0=error, 1=warning, 2=success, 3=info
[title_len: u16][title_bytes...]
[subtitle_len: u16][subtitle_bytes...]
```

Kind determines:

- Which builtin icon to render
- The accent color (left border + icon tint)

| Kind        | Icon             | Accent color |
| ----------- | ---------------- | ------------ |
| Error (0)   | error--solid     | `RED_60`     |
| Warning (1) | warning--solid   | `ORANGE_40`  |
| Success (2) | checkmark--solid | `GREEN_40`   |
| Info (3)    | info--solid      | `VIOLET_50`  |

**Visual structure** (informed by Carbon inline notification, Braiins frontend SCSS):

- 3px left border in accent color
- Dark background (`GRAY_90` or similar)
- 20×20 icon tinted with accent color, vertically centered
- Title text (bold, white) + subtitle text (normal, gray) — both optional
- 12px padding all around, 8px gap between icon and text
- Full width (fills parent), auto height

No close button for now (the user said irrelevant for now).

**Files:**

- `protocol/src/nodes.rs` — `NODE_NOTIFICATION: u8 = 0x08`

---

## 3. SDK: `notification()` builder function

Add to `sdk/src/tree.rs`:

```rust
#[derive(Clone, Copy)]
pub enum NotificationKind {
    Error = 0,
    Warning = 1,
    Success = 2,
    Info = 3,
}

pub fn notification(kind: NotificationKind, title: &str, subtitle: &str) -> Node {}
```

And the corresponding `Node::Notification` variant + `TreeBuffer::write_notification` + serialization.

**Files:**

- `sdk/src/tree.rs` — `NotificationKind` enum, `Node::Notification` variant, `notification()` fn, serialization
- `sdk/src/lib.rs` — re-export `NotificationKind` and `notification`

---

## 4. Host: deserialize + render notification

In `src/tree.rs`:

- Add `TreeNode::Notification` variant to the enum
- Deserialize `NODE_NOTIFICATION` in `read_node()`
- In `build_taffy_node()`: create a row node with fixed layout (icon + text column)
- In `render_taffy_node()`: render left border, background, icon (using builtin registry), title + subtitle text

The rendering uses existing primitives — `fill_rect` for border/background, `draw_icon` for the builtin icon,
`draw_text` for title/subtitle. No new renderer methods needed.

**Files:**

- `src/tree.rs` — `TreeNode::Notification`, deserialization, layout, rendering

---

## 5. SpaceX widget: error state

Update `examples/spacex-launch/src/lib.rs`:

- Change state from `Option<LaunchData>` to an enum: `Loading | Loaded(LaunchData) | Error(String)`
- On fetch failure (`!response.ok()`): set state to `Error` with a message, schedule retry
- In `render()`: match on state — show `notification(NotificationKind::Error, ...)` for error state
- Include the HTTP status in the error message when available (e.g. "API request failed (503)")

**Files:**

- `examples/spacex-launch/src/lib.rs` — state enum, error display, retry logic

---

## Files summary

| File                                | Action                                                  |
| ----------------------------------- | ------------------------------------------------------- |
| `icons/error--solid.svg`            | NEW — copy from carbon-icons                            |
| `icons/warning--solid.svg`          | NEW — copy from carbon-icons                            |
| `icons/checkmark--solid.svg`        | NEW — copy from carbon-icons                            |
| `icons/info--solid.svg`             | NEW — copy from carbon-icons                            |
| `build.rs`                          | Add 4 icon ID mappings                                  |
| `protocol/src/icon.rs`              | Add builtin icon ID constants                           |
| `protocol/src/nodes.rs`             | Add `NODE_NOTIFICATION = 0x08`                          |
| `sdk/src/tree.rs`                   | `NotificationKind`, `Node::Notification`, serialization |
| `sdk/src/lib.rs`                    | Re-export notification types                            |
| `src/tree.rs`                       | Deserialize + layout + render notification node         |
| `examples/spacex-launch/src/lib.rs` | Error state enum, notification display                  |

---

## Verification

1. `make dev EXAMPLE=spacex-launch` — normal operation, data loads, no notification visible
2. Temporarily break the API URL → widget shows error notification with red accent, icon, and message
3. After 30s retry, if API recovers → notification disappears, data loads normally
4. All 4 size variants render the notification correctly (text wraps, icon stays aligned)
5. `cargo clippy` — zero warnings
