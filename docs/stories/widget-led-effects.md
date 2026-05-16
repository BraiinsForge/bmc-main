# Widget LED Effects

Widgets — the configurable scenes a user can install and arrange — can drive the same 10-LED strip the system uses for
ambient notifications (see [LED Notifications](led-notifications.md) for the system-driven side). The system arbitrates
across all sources so widget effects feel scoped when they should, ambient when they should, and never bury an event the
user needs to see.

## User stories

### Scene-scoped widget effects

> As a user, I want a widget's LED effect to follow the scene I'm looking at, so widgets I'm not currently watching do
> not fight for the strip in the background.

- An effect started by a widget plays only while that widget's scene is the active scene.
- Swiping away from the scene mid-effect ends the visible portion of the effect; short, time-limited effects keep
  running their clock in the background so they finish on the schedule the widget asked for, but the user only sees
  whatever is on the currently active scene.
- Switching to a different scene shows that scene's widget effect (if any), or nothing.

### Ambient widget effects

> As a user, I want some widget effects to follow me across scenes, so a persistent signal like a pomodoro timer's
> running indicator stays visible no matter which scene I am on.

- Widgets can mark an effect as **ambient** (also called *global* in the developer protocol).
- Ambient effects render on the strip whenever no scene-local widget effect is competing for it.
- A scene-local effect always takes priority over an ambient one — the ambient effect waits and returns once the local
  effect ends or the user swipes away from its scene.

### System events always win

> As a user, I want device-level alerts to override widget effects so I never miss something important like a Wi-Fi
> reconnect or an alarm because a widget is showing a fancy pattern.

- Any effect from the [system LED notifications](led-notifications.md) ladder — boot, clock alarm, Wi-Fi state, firmware
  upgrade, scene preview, Wi-Fi scan, price alerts — displaces widget effects while it runs.
- When the system effect ends, the displaced widget effect resumes on the strip.

### Widget removal or crash clears its effects

> As a user, I want a widget's LED effects to stop when the widget is no longer running, so I never see a stuck
> animation left behind by a widget I removed or that crashed.

- Removing a widget from its scene clears any effects that widget had on the strip.
- If a widget crashes or otherwise disconnects, its effects clear automatically.

## Priority

Widget effects sit beneath the system layer in a fixed priority order (highest first):

1. System events (see [LED Notifications](led-notifications.md) for the internal ordering)
2. Scene preview
3. Widget effects — scene-local
4. Widget effects — ambient

Within either widget tier the most recent effect wins; the displaced effect (from the same or a different widget) is
cancelled and does not come back. Short, time-limited effects layer over a widget's persistent effect and revert to it
automatically when they expire, in the same spirit as the system layer's temporary flashes.

## Notes

- The master **Enable LED Notifications** toggle and the night-mode toggle described in
  [LED Notifications](led-notifications.md#enable--disable) apply equally to widget effects. When LEDs are disabled,
  widget effects render nothing.
- Any widget can currently request an ambient effect. Gating this through the widget manifest (so only widgets that
  legitimately need a cross-scene signal can use it) is expected as a follow-up.
