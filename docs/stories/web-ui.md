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
- `UpdateWidget` is a full-map update (not a patch): clients send the complete params object, and the backend validates
  required/missing keys, unknown keys, and per-type constraints (type/range/enum/timezone).
- Changing a parameter applies live to the running widget — the widget process keeps running and re-binds its state in
  place. Changes that affect the widget's size still respawn it (the only way to deliver a new geometry during the
  widget's initial configure batch). Position-only changes do not respawn.

### Preview configuration changes live

> As a user, I want to see my parameter changes on the device as I make them, so I can tell whether a setting looks
> right without committing it.

- Editing a widget's params or size pushes valid changes to the device with a short debounce, so the display reacts as
  the user types or toggles.
- By clicking on Add a widget either as a fullscreen scene or in combined, the coordinator creates the widget on the
  device immediately with its default params; the tile appears live and continues to reflect every form change.
- Cancelling add/edit dialog removes the widget so the scene is unchanged from before the dialog opened.
- Scene preview is exclusive: only one active preview stream is allowed at a time.
- Previewing a disabled scene temporarily spawns its widgets; ending preview tears those widgets down again.
- Live application explicitly shows an error to the user, but success presents no toast.

### Control widgets in real time

> As a user, I want to start, stop, and switch widgets from the browser and see the device react immediately.

- Stop or restart a widget by disabling and re-enabling its scene. There is no per-widget stop/restart RPC; widget
  lifecycle is driven by scene enabled state and by `UpdateWidget` (which respawns on size changes).

### Switch between scenes

> As a user, I want to switch the active scene from the browser, so I can preview or demo without walking to the device.

- Activate any enabled scene; the device transitions to it immediately.
- The currently active scene is clearly indicated in the UI.

### Adjust system-wide settings

> As a user, I want to change timezone, night mode, and localization from the browser, and have every widget on the
> device pick up the change without a restart.

- Timezone, night mode, and locale-related settings broadcast to every running widget via the `deck_widget_v1` Wayland
  protocol. Locale is delivered as separate events per field (date format, time format, number format, temperature unit,
  first day of week) rather than a single bundled "localization" event, so new locale fields can be added later without
  a breaking protocol change.
- Brightness is configured via the `ConfigurationService` gRPC API, not the Wayland broadcast — widgets don't see
  brightness as a setting event.
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
