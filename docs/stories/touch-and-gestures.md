# Touch & Gesture Input

The device has a 480x1280 capacitive touchscreen. Touch input lets users navigate between scenes and interact with
widgets.

## User stories

### Scene navigation

> As a user, I want to swipe left or right on the screen to switch between scenes, with the content following my finger.

- A horizontal drag should trigger a scene transition.
- The scene follows the finger position during the drag.
- On release, velocity and displacement determine whether the transition commits or snaps back.

### Widget interaction

> As a user, I want to tap and drag on widgets so they can respond to my input (e.g. sliders, buttons).

- Touch events are forwarded to the widget under the touch point.
- Coordinates are translated to the widget's local space (logical pixels, top-left origin).
- Widgets receive `touch_down`, `touch_motion`, and `touch_up` events.

## Protocol

Touch events flow through standard Wayland `wl_seat`/`wl_touch`, not the custom `deck_widget_v1` extension. The
compositor performs hit-testing, computes surface-relative coordinates, and sends `wl_touch.cancel` when it recognizes a
compositor-level gesture (scene navigation).

Action requests (sound, LED) flow through `deck_widget_v1`:

- **Widget to compositor:** `request_action(action_type, payload)`

## Constraints

- The touchscreen driver reports coordinates in logical landscape orientation. The compositor normalizes evdev axis
  ranges to logical pixel dimensions (no rotation needed).
- Single-touch only for the initial implementation.
