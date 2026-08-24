# Widget runtime configuration via `deck_widget_manager_v2`

## Description

A widget process receives its geometry, per-instance params, credentials, and current system settings over
`deck_widget_surface_v1`. It identifies the configured instance when creating that surface through the keyed
`deck_widget_manager_v2` factory.

A widget process's environment is:

```
WAYLAND_DISPLAY=<socket>
XDG_RUNTIME_DIR=<dir>
BMC_WIDGET_KEY=<canonical configured-widget UUID>
```

plus the coordinator's own environment passed through unchanged (`PATH`, `HOME`, locale, `RUST_LOG`, and anything else
the init system set up for BMC).

### Identity

`BMC_WIDGET_KEY` is the scene-level widget UUID. The manager keeps it as `WidgetConfigKey.instance_id`, separately from
`WidgetEnv`, which contains only the Wayland display. At each spawn, `WaylandSpawner` receives that UUID as its
`widget_key` argument and serializes it into `BMC_WIDGET_KEY`. The client supplies it to `get_widget_surface`. The
compositor accepts only when the key names an `Accepting` retained registration. The key is routing identity, not a
secret or an authentication boundary: all native widget components currently run as root and may read or reuse it.

The coordinator registers a configured instance before spawning its process. That registration survives child crashes,
ordinary stops, and upgrade pauses, and holds the latest placement and initial configuration. A registration is either
`Accepting` or `Inactive`. Deactivation closes the exact attached client but keeps the record; activation permits a
successor to attach. Deletion unregisters the record entirely. There is no PID association, generation stamp, or queue
of connections waiting for later registration. PID remains useful only for supervision and logs.

This is an intentionally breaking native protocol change. The compositor advertises only `deck_widget_manager_v2`, and
bundled clients require it; older clients using the former manager interface are not supported.

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
configure(width, height, viewport_shape, token)
display_info(width, height, shape, dpi)
params(json)
credentials(json) / credential_secrets(json)
timezone / night_mode / date_format / time_format /
  number_format / temperature_unit / first_day_of_week
configure_done
```

- `configure` carries the widget viewport's pixel dimensions, viewport shape, and an opaque per-instance token for
  namespacing resources such as caches.
- `display_info` carries the active logical display's pixel dimensions, display shape, and DPI. DPI is each platform's
  real display density and is advisory for layout.
- `params` carries the widget's per-instance params as a JSON-encoded object, exactly as stored in the scene config. The
  compositor passes it through as-is; the widget owns its manifest and is authoritative on what values are valid.
- `credentials` carries the resolved slot-to-account view, while `credential_secrets` carries the corresponding native
  process secrets and egress policy. Guest WASM never receives the secret event.
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
gracefully. The widget client surfaces this as a `Shutdown` event. Ordinary disable or replacement instead deactivates
the retained registration, closes its exact connection, and sends SIGTERM to the child. Deletion unregisters the record.
A child that does not exit within 10 seconds is force-killed with SIGKILL. The `shutdown` event is sent only when the
whole compositor shuts down.

#### Live previews

When the user is actively editing the configuration, each valid change sends config update to the widget even before
saving. The changes are debounced by 300 ms. Preview updates and committed updates use the same protocol events, so the
widget cannot distinguish them today.

The user might be changing multiple parameters subsequently. It is therefore advisable to the widget to debounce web
fetches or other expensive operations that are caused by parameter changes.

### Crash supervision

The widget manager owns every widget child process in a dedicated actor task and awaits its exit directly. A process
that exits on its own is respawned automatically: the delay starts at 1 second, doubles per crash up to 5 minutes, and
restarts from 1 second once a process stays up for 60 seconds. The compositor registration and stored configuration
survive the crash, so the successor supplies the same widget key and receives the latest initial batch. Process exit
does not produce a compositor registration update.

The 5-minute delay is a ceiling, not a restart budget. Supervision keeps retrying while the matching installed build is
available; if the type leaves the registry, the pending attempt ends and a later coordinator reload can start it again.
A budget would be unsafe because widget failures are correlated — a crashed `bmc-wasm-host` drops the control socket of
every thin at once, so one host fault exits the whole wasm fleet together, and a per-widget budget would be spent by a
fault no individual widget caused. The 60-second healthy threshold sits above the thin's own startup budget
(`DEFAULT_HOST_WAIT` + `DEFAULT_ACK_WAIT`), so a widget that never once reached its host cannot be mistaken for a
healthy one and keep resetting the ladder.

Each instance is exactly one of `Running`, `Stopping`, or `PendingRestart`. Pending state stores the launch record,
installed build identity, earned backoff rung, and an internal timer token. Replacing or removing that state aborts the
timer; a timer event already queued is harmless unless its token still names the current pending entry. Before spawning,
the manager also checks that it is `Running` and that the registry still exposes the same build identity. A registry
identity mismatch preserves the pending launch for the coordinator's guarded reload path; it never starts a new build
through state prepared for the old one.

An external stop cancels pending state. A running child becomes `Stopping` until its exit is consumed, and a successor
cannot start over it. Disable, preview teardown, deletion, upgrade pause, and terminal shutdown all use this path. A
later enable or preview open therefore starts fresh; a stale timer cannot revive the previous run.

A registry re-scan brings pending timers forward to 1 second but preserves every earned rung. The timer still requires
the stored build identity to match the installed entry; the coordinator handles a build change with its normal
stop-before-start replacement. A widget the upgrade did not fix therefore continues from the backoff it already earned.

A semantic parameter change updates the retained configuration even when no child is attached. A successful parameter
enqueue asks only that instance to retry promptly. Widget binding and account changes wake the broad credential
listener. The listener re-resolves every configured widget. Credential updates have a bounded receipt whose result says
whether the retained view or secrets actually changed; only a changed result accelerates that instance. The earned rung
survives either acceleration, and running, stopping, or absent instances are untouched.

A coordinator start that observes a matching `PendingRestart` is already satisfied and leaves its timer alone. This is
important for repeated reload and enable work: duplicate starts do not continually bring a crash loop forward. Only the
explicit targeted retry paths and registry refresh reschedule a pending timer. If the manager deliberately receives a
new validated start for a pending instance, it cancels the old timer and attempts that launch immediately; a second
failure preserves the earned rung and leaves exactly one replacement timer.

### Mutation and receipt ordering

The compositor retains registration state independently of the child. Registering an existing key updates that record;
it does not disconnect an attachment. Operations that require a restart explicitly deactivate and stop first, then
register the current configuration, activate, and start the successor.

Registration and activation are synchronously enqueued under the authoritative configuration source locks. Their
receipts are awaited only after those locks are released, and configuration is revalidated before the manager start is
sent. Before activation, the coordinator obtains an actor-owned permit scoped to the expected manager mode and passes it
to the later `Spawn`. Deactivation and unregister use the same rule: enqueue the compositor cutoff and manager stop
under the source lock, then await the receipt and child reap without that lock. Every stop invalidates the instance's
permit, so an earlier start cannot cross the later cutoff even when both operations used configuration read locks. A
two-second receipt timeout is reported, but never prevents stopping and reaping the child.

The complete lock order is preview source before configuration before secrets. A path takes only the suffix it needs;
code holding a secret-store lock never reaches back for configuration or preview. Parameter and credential updates are
enqueued while their authoritative values are locked. FIFO ordering either updates an existing retained record or lets a
following registration carry the same values. No compositor receipt or child termination is awaited while holding
configuration or secret-store locks. Preview teardown retains the preview lock through the compositor cutoff receipt and
child termination, then restores scene cycling before releasing the slot. This can serialize preview reopening, widget
starts, upgrade resume, and scene or widget configuration RPCs for the child's ten-second graceful shutdown period, but
prevents a successor preview from being overwritten by the preceding teardown.

The broad credential listener subscribes to scene and account changes. After either notification it coalesces queued
notifications, takes configuration then secret-store read locks, and resolves every distinct configured widget from one
snapshot. It enqueues updates for all resolvable retained records, including inactive or detached ones, then drops both
locks before awaiting the bounded receipts concurrently. The compositor reports whether each retained credential view or
secret set changed semantically, and only that result accelerates the corresponding pending widget. An unavailable
manifest skips that widget rather than trusting stale bindings; it does not stop refreshes for siblings.

Account persistence runs in a detached task, so request cancellation cannot interrupt an accepted save or deletion.
Successful non-idempotent account saves notify the listener. Idempotent updates return before saving and do not emit a
notification. Handlers do not select an affected subset, and no persistent credential retry queue is added.

### Upgrade pause and resume

Firmware upgrade first moves the manager to `Paused`. This rejects new starts, cancels every pending restart, and makes
later exit reports unable to schedule successors. Under the configuration write lock, the coordinator enqueues
deactivation for every configured or supervised instance. It then releases the lock and concurrently waits for all
deactivation receipts and child terminations; a compositor timeout cannot leave a widget process running.

Resume is the one flow allowed to enqueue activation while the manager remains `Paused`. It takes the preview lock and
then the configuration read lock, derives the supported shown set (`enabled || actively previewed`), and enqueues only
activation for those retained records after preparing start permits scoped to `Paused`. It neither rebuilds
registrations nor takes the secret-store lock. After awaiting activation receipts without source locks, it reacquires
preview then configuration read locks. It revalidates the same shown set, spawn prerequisites, installed build
identities, and `Paused` mode. Only then does it move the manager to `Running`, preserving the paused permits, and
enqueue current starts with them before releasing the locks. Stop, pause, and shutdown invalidate prepared permits; each
Spawn consumes only its exact match. A failed or cancelled upgrade uses this same guarded resume. Terminal
`ShuttingDown` absorbs a late resume, so it cannot activate or spawn widgets after application shutdown has begun.

### What supervision observes — and what it does not

Detection is a chain of four edge-triggered hops, with no polling at any layer:

1. A wasm guest trap, a host-side panic, or `max_fuel_strikes` consecutive fuel-outs makes `slot.render` return an error
   or `RenderStatus::Dead`. The host sees this as an ordinary return value — it *is* the interpreter running the guest,
   so there is no boundary to poll across.
2. The host tears the slot down. `WidgetSlot` owns the thin's control socket, so dropping the slot closes that fd.
3. The thin, parked in `poll(2)` on the control socket and a signal pipe, wakes on `POLLHUP` and exits 0.
4. The kernel raises `SIGCHLD` in bmc, the thin's direct parent; `child.wait()` returns and the actor treats the exit as
   a crash if the instance is still `Running`. PID is checked only against the actor-owned child state.

"Crash" is therefore never detected, only inferred: it means *the process died and we never asked it to*. An external
stop moves the instance to `Stopping` before signalling, so the later exit completes termination instead of scheduling a
restart.

Two consequences worth knowing:

- **Exit status carries no policy signal.** A thin exits 0 when its slot goes away — including when the host dies, which
  is the normal fleet-wide failure — and non-zero for a bad manifest path *and* for a transient Wayland connect failure.
  Supervision therefore ignores exit status entirely.
- **Only process death is observed.** Fuel metering does catch a compute-wedged widget (it escalates to `Dead`, which
  tears the slot down), but a widget that returns promptly while being logically stuck — waiting on a fetch that never
  resolves, re-rendering a stale frame — is indistinguishable from a healthy one. Neither the Wayland connection state
  nor any heartbeat feeds supervision.

## Constraints

- The widget process must block on `configure_done` before rendering; without `configure`, `width` and `height` are
  unknown and the renderer can't be sized. A 10 s timeout guards against a dead compositor.

- Registration and activation receipts must complete before spawning a child, so a fast client can never reach the
  compositor before its accepting retained record exists. A failed or timed-out receipt prevents that spawn.

- On-disk config (`/etc/bmc_config.json`) is unchanged. The widget instances carry their params in the scene config; the
  compositor serves them over the protocol.

- Params are typed end-to-end before they reach Wayland. The scene config, compositor, and Wayland-side serialization
  run on `serde_json::Map<String, Value>`. The widget process receives params as a JSON-encoded string (Wayland's
  `params(json)` event) and deserializes via its own `#[derive(Deserialize)]` schema — the manifest is the source of
  truth on what the widget expects, and the pipeline carries the user's choices through without ad-hoc per-layer string
  parsing.
