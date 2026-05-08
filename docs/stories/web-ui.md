# Web UI for Deck Configuration

A browser-based configuration interface hosted by the device itself. The Web UI covers scene and widget management for
the Wayland-era Deck, preserving feature parity with the pre-Wayland web app while dropping hardcoded widget types in
favor of a manifest-driven model — users can configure any installed widget, including out-of-tree ones, without a
firmware rebuild.

## User stories

### Manage scenes

> As a user, I want to manage scenes from a web browser, so I can set up my Deck without using the on-device touch UI
> for config-heavy tasks.

- Create, edit, and delete scenes.
- Choose the scene layout: **fullscreen** (one `full`-sized widget) or **combined** (multi-widget grid of `small` /
  `medium` / `large` widgets).
- Rearrange widget positions within combined scenes.
- Enable or disable scenes; configure the scene cycling interval.
- Changes persist and survive device reboots.

### Add widgets from a live catalog

> As a user, I want to browse and place any widget my device has installed, not a hardcoded list, so new widgets are
> usable as soon as they ship.

- The widget picker shows every manifest the device currently has installed — no fixed enum.
- Widget size options are scene-aware and manifest-driven: combined scenes offer Small / Medium / Large; fullscreen
  scenes use Full.
- Place a widget at a grid position within a combined scene; the dropped widget appears on the device immediately.

### Configure widget settings

> As a user, I want widget-specific settings to appear as a form I can fill in, without each widget needing its own
> bespoke config screen.

- Config forms are generated from the widget's manifest parameter schema — string, enum, boolean, number, and timezone
  fields.
- No per-widget form components exist in the frontend; a new parameter kind added once benefits every widget.
- Changing a parameter restarts the widget with the new config automatically.

### Control widgets in real time

> As a user, I want to start, stop, and switch widgets from the browser and see the device react immediately.

- Stop or restart a single widget from the UI.

### Switch between scenes

> As a user, I want to switch the active scene from the browser, so I can preview or demo without walking to the device.

- Activate any enabled scene; the device transitions to it immediately.
- The currently active scene is clearly indicated in the UI.

### Adjust system-wide settings

> As a user, I want to change timezone, night mode, and localization from the browser, and have every widget on the
> device pick up the change without a restart.

- Timezone, night mode, and localization broadcast to every running widget via the Wayland protocol.
- Brightness control.
- Changes apply immediately — no widget restart required (settings delivery is handled by the `deck_widget_v1`
  protocol).

## Constraints

- **Feature parity** with the pre-Wayland web UI is the baseline for scene and widget management.
- **Widget settings are manifest-driven**, not hardcoded in the frontend.
- **No dependency on the old monolith** — `DisplayController`, `WidgetTasks`, and the `WidgetKind` enum are not part of
  the new configuration flow.

## Out of scope

- Firmware upgrade flow — separate story area.
- Initial-setup wizard — separate story area.
- Alarm / scheduler configuration — separate story area.
- Account management — separate story area.
- Auth can be added later; the current focus is scene/widget configuration surface.
