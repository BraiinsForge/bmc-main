# Frame Scheduling Models

## 1. Widget-driven (current)

Widget calls `request_frame()` / `request_frame_after(ms)`.

- Zero cost when idle, which is most of the time for a dashboard widget
- Widget knows best when it needs to update (animation tick, data arrived, etc.)
- But the host already has to intervene for interactions anyway (scroll, button hover)
- And the "forgot to call `request_frame()`" bug is a real footgun for widget devs

## 2. Host-driven fixed rate

Host calls `render(delta_ms)` at N fps unconditionally.

- Simpler widget code — just render state, never think about frame scheduling
- Host can enforce a time budget: "you get 8ms per frame, 4 widgets × 8ms = 32ms budget at 30fps"
- Interactions, scroll, animations all just work — no special paths
- But wastes CPU on idle widgets, which matters on ARM

## 3. Hybrid (recommended)

Host drives at fixed rate, but widget can opt into sleep.

- Host ticks at e.g. 30fps, but if `render()` returns "nothing changed" the host skips compositing
- Widget still doesn't need to think about `request_frame()` for interactions
- Widget CAN signal "I'm idle, skip me until I get a WS message or touch event"
- Host controls the budget ceiling, widget controls the floor

For our case — embedded ARM, multiple widgets sharing a display, some interactive, some data-driven — option 3 fits
best. The host owns the frame clock and the budget. Widget devs never need to think about `request_frame()`. The idle
optimization comes from the return value of `render()` rather than from explicit frame requests.

The frame rate itself could be adaptive: 30fps when any widget is animating, drop to 10fps or even 1fps when everything
is idle. The host is in the best position to make that call since it sees all widgets.

The `request_frame_after(ms)` API could stay as a hint — "I have an animation, please don't drop below 30fps for the
next 500ms" — rather than the primary scheduling mechanism.
