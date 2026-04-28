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
the init system set up for BMC). An allowlist was considered and rejected: curating which variables a widget may see is
prone to omitting a variable the system actually needs (`TZ`, locale, resolver settings, future additions), with no
clear payoff — widgets already run as the same user as the coordinator and see its filesystem, so filtering env alone
doesn't establish a meaningful privilege boundary.

### Identity

Each widget process is associated with its scene-level `instance_id` by the compositor using the peer credentials of the
Wayland connection (`SO_PEERCRED`). The coordinator tells the compositor "pid X is instance Y" immediately after
spawning; the compositor resolves the connection to its instance when the widget's first `get_widget_surface` request
arrives.

Connections that arrive before the coordinator registers the pid are buffered in the compositor and resolved as soon as
the registration lands. When a widget process exits the coordinator clears the pid association so a recycled OS pid
cannot be mistaken for the dead widget.

### Initial configuration

On `get_widget_surface` the compositor emits a batch of events terminated by `configure_done`:

```
configure(size_type, width, height)
params(json)
timezone / night_mode / date_format / time_format /
  number_format / temperature_unit / first_day_of_week
configure_done
```

- `configure` carries the widget's size class and pixel dimensions.
- `params` carries the widget's per-instance params as a JSON-encoded object, exactly as stored in the scene config. The
  compositor passes it through as-is; the widget owns its manifest and is authoritative on what values are valid.
- Setting events carry the current system-wide values. The compositor keeps these cached so every newly connected widget
  starts with a fully populated state.
- `configure_done` tells the widget the batch is finished and it can start rendering.

### Runtime updates

After `configure_done` the same setting events continue to flow whenever a system setting changes. `params` is only
emitted once per connection — a change to a widget's params requires respawning the widget.

## Constraints

- The widget process must block on `configure_done` before rendering; without `configure`, `width` and `height` are
  unknown and the renderer can't be sized. A 10 s timeout guards against a dead compositor.

- `register_widget` and `set_widget_pid` are synchronous (flume ack); the coordinator waits on them so the compositor
  has the record before it's asked to resolve a connection. A connection that still races past the pid registration is
  buffered rather than dropped.

- On-disk config (`/etc/bmc_config.json`) is unchanged. The widget instances carry their params in the scene config; the
  compositor serves them over the protocol.
