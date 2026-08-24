# Widget Lifecycle

Widget lifecycle has three related parts:

- a stable configured-instance key routes a widget process to its compositor record;
- the coordinator and widget manager control whether that instance may run and connect;
- `deck_widget_surface_v1.lifecycle` describes where an attached widget is in the displayed scene.

For widget geometry, params, credentials, settings, and the initial configure batch, see
[`widget-runtime-configuration.md`](widget-runtime-configuration.md). For how the WASM host turns scene lifecycle events
into buffer and rendering policy, see [`wasm-host/render-loop.md`](wasm-host/render-loop.md).

## Configured-Instance Key

Every configured widget has a UUID that remains stable while that configured instance exists. The coordinator uses it as
`WidgetInstanceKey` and registers the instance with the compositor. The manager retains the same UUID in the launch's
`WidgetConfigKey`; `WaylandSpawner` receives it separately from `WidgetEnv` and writes it to the child's
`BMC_WIDGET_KEY`. The widget supplies the key to `deck_widget_manager_v2.get_widget_surface`, and the compositor
attaches the new surface to the matching retained registration.

The key is routing identity, not a security credential. It is intentionally available to the widget process and only
selects an already registered instance. The compositor accepts it only while that registration is in `Accepting` mode.
It must not be treated as a secret or as authentication between mutually untrusted processes.

Registration precedes process spawn, so a fast child can connect immediately. There is no process-registration handshake
after spawn and no connection waiting area: a connection either presents a canonical key for an accepting retained
record or receives a protocol error.

## Retained Compositor Registration

The compositor keeps one retained record per configured-instance key. The record contains placement and initial
configuration independently of any attached process and has one of two connection modes:

| Mode        | Meaning                                                                                     |
| ----------- | ------------------------------------------------------------------------------------------- |
| `Accepting` | a process presenting this key may attach; a new attachment replaces the current attachment  |
| `Inactive`  | connection attempts are rejected, while placement and initial configuration remain retained |

Registering an existing key refreshes its placement and initial configuration without implicitly changing its connection
mode. `ActivateWidget` changes the retained record to `Accepting`. `DeactivateWidget` changes it to `Inactive`, detaches
its protocol surface, `wl_surface`, and client, purges queued actions and connection notifications for that attachment,
forgets emitted scene-lifecycle state, drops compositor render state, and closes the attached Wayland client connection.
If an attachment existed, the compositor then records one disconnection notification for the cutoff.

`UnregisterRetainedWidget` performs the same attachment and render cleanup and removes the retained record itself.
Deactivation is used when an instance may return, including ordinary replacement and upgrade pause. Unregistration is
reserved for deleting the configured widget or scene.

Only one attachment is current for an instance. When another client attaches to an accepting record, the compositor
detaches and closes the previous client before installing the new attachment. Surface destruction identifies both the
client and protocol-surface object, so late destruction of an old attachment cannot detach its replacement. Requests
from a stale attachment are ignored.

## Process Manager

`WidgetManager` is the sole owner of child-process state. Each managed instance is `Running`, `Stopping`, or
`PendingRestart`, and the manager itself has a separate mode:

| Manager mode   | Starts and retries | Meaning                                                                  |
| -------------- | ------------------ | ------------------------------------------------------------------------ |
| `Running`      | allowed            | normal operation                                                         |
| `Paused`       | rejected           | temporary upgrade cutoff; an explicit resume may return to `Running`     |
| `ShuttingDown` | rejected           | terminal cutoff; resume is absorbed and leaves the manager shutting down |

Pause and shutdown publish the new manager mode before draining child state. Running children begin termination,
already-stopping children keep the same termination handle, and pending restart timers are cancelled. This ordering
makes a concurrent start observe the cutoff before any stop or compositor acknowledgement can complete.

An explicit stop sends `SIGTERM`, waits up to ten seconds for cleanup, then uses `SIGKILL` if necessary and reaps the
child. Stopping a `PendingRestart` instance cancels its timer and completes immediately.

### Crash supervision and retry

An unexpected exit in `Running` mode schedules another spawn and retains the compositor registration. The delay begins
at one second, doubles after repeated short-lived failures, and is capped at five minutes; supervision does not give up
at the cap. A process that ran for at least one minute is considered healthy, so its next failure starts a new
one-second ladder.

Spawn failures also become `PendingRestart` when the selected widget build is still installed. Each retry rechecks the
registry identity before executing it. If the build changed, the retry remains pending at its current rung until the
coordinator's guarded reload installs a matching launch. A registry refresh may bring the timer forward, but it cannot
substitute a different build identity.

A registry refresh, parameter change, or effective credential change can ask a pending instance to retry promptly. That
moves its next attempt to the initial delay without forgiving the backoff rung already earned. The prompt is a no-op for
running, stopping, or explicitly stopped instances.

## Coordinator Ordering

The coordinator connects configuration, compositor registration, and process supervision. Its complete lock order is
preview source before configuration before the secret store. A path takes only the suffix it needs; code holding a later
lock never reaches back for an earlier one.

Lifecycle commands that must be ordered with a configuration mutation are enqueued while the relevant configuration lock
is held. The coordinator does not wait for compositor receipts or child termination while holding that lock. It drops
the lock, waits, then reacquires and revalidates configuration and registry identity before spawning. This preserves
serialization without allowing a slow compositor or child to block configuration access.

Widget-manager actor round-trips may occur under configuration locks. Manager command handling must therefore remain
independent of the preview, configuration, and secret-store locks.

### Start and replacement

A configured instance owns a retained registration even while disabled, unpreviewed, paused, or detached. Visibility
controls activation and process eligibility, not registration lifetime. A normal start for a shown widget follows:

1. Under preview then configuration locks, read the configured widget, shown state, and current registry identity.
2. Ask the manager actor for a start permit scoped to `Running`, then refresh the retained registration and enqueue
   activation while configuration is stable.
3. Drop the configuration lock and wait for both compositor receipts.
4. Reacquire the lock and verify that scene visibility, widget spawn prerequisites, registry identity, and manager mode
   are unchanged.
5. Enqueue the process start with the same permit while that verified configuration is still locked, then drop the lock
   before waiting for the spawn result.

Every stop invalidates the prepared permit for that instance, including a stop issued before any child exists. `Spawn`
consumes only its matching permit, so a start whose activation was followed by a later cutoff returns `Superseded`
instead of crossing that cutoff. A newer preparation also supersedes the older permit without letting the old caller
consume the new one.

If the manager already owns the same launch and registry identity, the start is already satisfied. If it owns a
different launch or build for the same configured key, replacement is strictly stop-before-start:

1. Enqueue deactivation and request process stop while the current configuration is locked.
2. Drop the lock and wait for both attachment cutoff and child termination.
3. Repeat preparation against current configuration, refresh and reactivate the retained registration, and only then
   spawn the replacement.

The old and new processes therefore never intentionally overlap for one configured instance.

## Upgrade Pause and Resume

Before an upgrade can replace runtime files, the coordinator calls `begin_pause`. The manager becomes `Paused` before
any child is stopped, so reload paths and respawn timers cannot start widgets during the cutoff. While holding the
configuration write lock, the coordinator enqueues deactivation for every configured key and every managed key returned
by the pause transition. It then drops the lock and waits concurrently for all deactivation receipts and all child
termination handles. Registrations remain retained but inactive.

The upgrade run gate remains held through recovery, so another upgrade cannot overlap pause with an unfinished resume.
Normal failure awaits recovery before releasing that gate; cancellation uses the armed recovery guard to start the same
resume path.

Resume is a two-phase activation-only operation:

1. Under preview and then configuration read locks, derive the supported shown set. Prepare permits scoped to `Paused`
   and enqueue activation for existing retained registrations. Resume does not refresh registrations or take the
   secret-store lock.
2. Drop the lock and wait for all receipts concurrently.
3. Reacquire preview and then configuration read locks and verify that the full shown widget set, spawn prerequisites,
   registry identities, and `Paused` mode still match the prepared snapshot.
4. If anything changed, prepare again; otherwise publish `Running` without clearing the paused permits, refresh scene
   state, and enqueue starts with those permits before dropping the lock.
5. Wait for spawn results without holding the configuration lock.

The compositor can accept connections only after activation, and the manager can spawn only after the second-phase
validation with a matching permit. Pause and shutdown clear all prepared permits; resume preserves only the permits
prepared during its `Paused` phase. This prevents widgets from returning from stale pre-upgrade configuration.

## Terminal Shutdown

Terminal application shutdown begins after the web service and configuration-reload task have finished. The coordinator
changes the manager to `ShuttingDown`, which cannot be resumed, then enqueues final deactivation of all configured and
managed keys under the configuration write lock. After dropping the lock, it waits concurrently for attachment cutoffs
and child reaping. Only then does it shut down the compositor, keeping compositor-side cleanup available until every
attachment is cut off and every child is reaped.

If shutdown races an upgrade recovery, `ShuttingDown` absorbs the resume and terminal deactivation is the final
connection-mode command. No widget can spawn after the terminal mode is published, and no attachment remains accepted
after the final deactivation phase.

## Scene Lifecycle Events

Once attached, a widget receives `deck_widget_surface_v1.lifecycle(state)` events describing its relationship to the
displayed scene:

| State      | Meaning                                                               |
| ---------- | --------------------------------------------------------------------- |
| `Dormant`  | off-screen; a render target should not be needed                      |
| `Prepared` | an immediate scene-cycling neighbour that may pre-render              |
| `Entering` | the drag-direction or automatic-transition neighbour moving on-screen |
| `Visible`  | active on-screen while no transition moves it out                     |
| `Leaving`  | the active widget moving off-screen                                   |

The initial configure batch is emitted when the keyed surface attaches, followed by an initial idle lifecycle state. An
initial `Entering` is clamped to `Prepared`, and an initial `Leaving` is clamped to `Visible`, because a new client has
no prior transition history.

Regular committed scene changes defer lifecycle emission until after the compositor renders the committed frame. Drag
motion and automatic pre-transition emit non-releasing transitions immediately so widget rendering stops before it can
contend with transition rendering.

Transitions into `Dormant` form a release batch. The compositor sends dormant transitions and releases held buffers,
flushes those clients, and only then sends and flushes acquire transitions into the other states. Clients should treat
every lifecycle event as current truth and tolerate repeated states or skipped intermediate states.

## Code Map

- `bmc/src/widget/manager.rs` - process ownership, modes, crash supervision, and termination.
- `bmc/src/widget/coordinator.rs` - configuration ordering, retained registration, replacement, pause, and resume.
- `bmc/src/widget/spawner.rs` - configured-instance key environment setup.
- `bmc/src/compositor.rs` - retained registration and connection-mode command types.
- `bmc-openwrt/src/compositor/protocol/dispatch.rs` - keyed surface admission and stale-attachment request handling.
- `bmc-openwrt/src/compositor/protocol/state.rs` - retained records and exact attachment cleanup.
- `bmc-openwrt/src/compositor/state.rs` - client cutoff and renderer/lifecycle cleanup.
- `bmc-openwrt/src/compositor/lifecycle_emitter.rs` - scene lifecycle release/acquire batches.
- `bmc-widget-protocol/protocol/deck-widget.xml` - keyed manager and scene lifecycle wire contract.
