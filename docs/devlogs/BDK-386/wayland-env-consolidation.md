# Consolidate Settings Delivery via Wayland Protocol

## Problem

Timezone, night mode, and localization were delivered to widgets through two channels:

- **Spawn-time env vars** (`DECK_TIMEZONE`, `DECK_NIGHT_MODE`, `DECK_LOCALIZATION`) read once by the widget during boot.
- **Runtime Wayland events** via `deck_widget`'s `setting` message, delivered whenever a user changes a setting.

Two code paths, one data model — easy to drift out of sync, and widgets had to read the env var and register the Wayland
handler to cover both first-frame and live updates.

## Fix

Drop the env-var channel. Settings arrive exclusively through the Wayland `setting` event, sent on two occasions:

1. **On connect.** The compositor caches the current `SettingUpdate` values and calls `send_setting_to_widget()`
   immediately when a widget binds `deck_widget` — before the widget renders its first frame.
2. **On change.** The coordinator broadcasts updated settings through `CompositorCommand::BroadcastSetting`, same as
   before.

### Boundary — what stays in env vars

Identity and geometry information that the widget needs *before* the Wayland connect handshake stays in env vars:

| Env var            | Reason to keep                                 |
| ------------------ | ---------------------------------------------- |
| `DECK_INSTANCE_ID` | Widget identifies itself when binding protocol |
| `DECK_SIZE_TYPE`   | Determines buffer dimensions at init           |
| `DECK_WIDTH`       | Allocate framebuffer before first commit       |
| `DECK_HEIGHT`      | Same                                           |
| `DECK_PARAMS`      | Widget-specific config baked in at spawn       |

These are required before the widget can request a `wl_surface`, so moving them to a Wayland event would create a
chicken-and-egg problem.

## Data flow

```
startup
  coordinator.broadcast_initial_settings()
    → compositor.cache_settings(SettingUpdate)
      → stored in DeckWidgetProtocolState

widget spawn
  spawner → exec widget (with identity/geometry env only)
    → widget binds deck_widget
      → compositor.send_setting_to_widget(instance_id, cached)
        → widget.setting(type, value) for each cached setting

user changes timezone
  coordinator.broadcast_timezone("CET")
    → compositor.broadcast_setting(Timezone("CET"))
      → send setting event to every widget
```

## Changes

- `bmc-openwrt/src/compositor/egl_compositor.rs` — cache settings, send on connect.
- `bmc-openwrt/src/compositor/protocol/state.rs` — add `send_setting_to_widget()`.
- `bmc-widget/src/env.rs` — remove `read_settings()` and the `DECK_TIMEZONE`/`DECK_NIGHT_MODE`/`DECK_LOCALIZATION`
  consts.
- `bmc/src/widget/coordinator.rs` — call compositor cache on startup; broadcast helpers simplified.
- `bmc/src/widget/spawner.rs` — drop the three settings env vars; `WidgetEnv` and `spawn_widget()` signatures
  simplified.
- `widgets/digital-clock/src/ipc.rs`, `widgets/flip-clock/src/ipc.rs` — remove dead `read_settings()` calls (they were
  silently returning empty values since the env vars were no longer set during the migration).

## Migration note

Any widget still reading `DECK_TIMEZONE` / `DECK_NIGHT_MODE` / `DECK_LOCALIZATION` directly from the environment will
get `None` / empty values. The fallback is the Wayland `setting` handler — which all in-tree widgets already have — so
out-of-tree widgets need to wire that handler before upgrading.
