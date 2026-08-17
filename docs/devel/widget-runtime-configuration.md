# Widget runtime configuration via `deck_widget_v1`

## Description

A widget process receives everything it needs to run — its geometry, its per-instance params, and the current system
settings — over the `deck_widget_v1` Wayland protocol. Nothing BMC-specific appears in the child's environment.

A widget process's environment is:

```
WAYLAND_DISPLAY=<socket>
XDG_RUNTIME_DIR=<dir>
```

plus the coordinator's own environment passed through unchanged (`PATH`, `HOME`, locale, `RUST_LOG`, and anything else
the init system set up for BMC).

### Identity

Each widget process is associated with its scene-level `instance_id` by the compositor using the peer credentials of the
Wayland connection (`SO_PEERCRED`). The coordinator tells the compositor "pid X is instance Y" immediately after
spawning; the compositor resolves the connection to its instance when the widget's first `get_widget_surface` request
arrives.

Connections that arrive before the coordinator registers the pid are buffered in the compositor and resolved as soon as
the registration lands. When a widget process exits the coordinator clears the pid association so a recycled OS pid
cannot be mistaken for the dead widget.

### Parameters

Each widget declares parameters it needs in its `manifest.json`. The coordinator itself ensures that the parameters will
be sent to the widget according to the manifest at the time the widget has been added to a scene by the user. Even
required values are always populated with the default, so the widget does not have to handle default values itself.

There is one caveat, though. When the widget changes its `manifest.json`, the old variant will be sent to the widget,
until the user has updated the config. This case has to be handled more robustly in the future, but it is not right now.
So in case the widget decides to update the manifest, it has to handle it on its own. It has to handle both the old and
new versions, migrating the parameters in memory. Or crash explicitly if it can't migrate (that cannot be encouraged,
though, showing something is usually preferable to nothing at all, the user will adjust if they see something wrong)

### Initial configuration

On `get_widget_surface` the compositor emits a batch of events terminated by `configure_done`:

```
configure(width, height, viewport_shape)
display_info(width, height, shape, dpi)
params(json)
timezone / night_mode / date_format / time_format /
  number_format / temperature_unit / first_day_of_week
configure_done
```

- `configure` carries the widget viewport's pixel dimensions and viewport shape.
- `display_info` carries the active logical display's pixel dimensions, display shape, and DPI. DPI is each platform's
  real display density and is advisory for layout.
- `params` carries the widget's per-instance params as a JSON-encoded object, exactly as stored in the scene config. The
  compositor passes it through as-is; the widget owns its manifest and is authoritative on what values are valid.
- Setting events carry the current system-wide values. The compositor keeps these cached so every newly connected widget
  starts with a fully populated state. Each locale-related field (date / time / number format, temperature unit, first
  day of week) is a separate event rather than a single bundled "localization" event, so new locale fields can be added
  later as additional events without a breaking protocol change.
- `configure_done` tells the widget the batch is finished and it can start rendering.

### Runtime updates

After `configure_done` the same setting events continue to flow whenever a system setting changes. The `params` event is
also re-emittable: when widget params change without a size change, the compositor pushes a fresh JSON-encoded params
blob on the existing surface instead of stopping and respawning the widget process. Even on position changes, the widget
does not respawn as positioning is a compositor concern, not a widget concern. The widget client surfaces the
post-`configure_done` `params` event as a separate runtime event (`ParamUpdate`) so per-widget code can re-bind its
state in place — plain Rust state plus a needs-render flag for the flip-clock, etc.

Changes that *do* affect viewport size or shape still respawn — the widget only receives `configure(...)` and
`display_info(...)` once, during the initial batch, so a new geometry needs a fresh process to size its renderer for.

Runtime param pushes are full replacements of the widget params map (not partial patches). Widgets should treat every
`params(json)` event as a complete snapshot and re-bind state from that snapshot.

### Shutdown

When the compositor itself is exiting it broadcasts a `shutdown` event to every connected widget so they can exit
gracefully. The widget client surfaces this as a `Shutdown` event. When the widget is being removed from the scene, it
is killed by SIGTERM; if it does not exit within 10 seconds, it is force-killed with SIGKILL. The `shutdown` event is
sent only when the whole compositor shuts down.

#### Live previews

When the user is actively editing the configuration, each valid change sends config update to the widget even before
saving. The changes are debounced by 300 ms. Preview updates and committed updates use the same protocol events, so the
widget cannot distinguish them today.

The user might be changing multiple parameters subsequently. It is therefore advisable to the widget to debounce web
fetches or other expensive operations that are caused by parameter changes.

### Crash supervision

The widget manager owns every widget child process in a dedicated actor task and awaits its exit directly. A process
that exits on its own is respawned automatically: the delay starts at 1 second, doubles per crash up to 30 seconds, and
resets after the process stays up for 60 seconds. The instance's compositor registration and stored configuration
survive the crash, so the respawned process attaches through the same configure replay as the first spawn; its new pid
is re-bound via `set_widget_pid`, and a connection racing past that registration is buffered as usual.

An external stop always wins: stopping a widget (scene edit, upgrade preparation, shutdown) cancels a pending respawn,
and a stopped widget is never respawned. A widget whose type has left the registry (uninstalled) is not respawned either
— its grid cell stays empty, exactly as if it had never spawned.

## Constraints

- The widget process must block on `configure_done` before rendering; without `configure`, `width` and `height` are
  unknown and the renderer can't be sized. A 10 s timeout guards against a dead compositor.

- `register_widget` and `set_widget_pid` are synchronous (flume ack); the coordinator waits on them so the compositor
  has the record before it's asked to resolve a connection. A connection that still races past the pid registration is
  buffered rather than dropped.

- On-disk config (`/etc/bmc_config.json`) is unchanged. The widget instances carry their params in the scene config; the
  compositor serves them over the protocol.

- Params are typed end-to-end before they reach Wayland. The scene config, compositor, and Wayland-side serialization
  run on `serde_json::Map<String, Value>`. The widget process receives params as a JSON-encoded string (Wayland's
  `params(json)` event) and deserializes via its own `#[derive(Deserialize)]` schema — the manifest is the source of
  truth on what the widget expects, and the pipeline carries the user's choices through without ad-hoc per-layer string
  parsing.
