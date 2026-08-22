# Widget Lifecycle Simplification

> Historical design record for the BDK-406/BDK-747 implementation. See
> [Widget lifecycle](../../devel/widget-lifecycle.md) for the maintained architecture.

## Context

`WidgetManager` owns widget child processes, observes their exits, and applies restart backoff. The compositor currently
identifies a widget connection by PID and also participates in process supervision by clearing and rebinding PIDs. That
split requires registration generations, tombstones, guarded bind operations, and pending connections for processes that
reach Wayland before their PID is bound.

A compositor registration describes a configured widget instance, not a particular process incarnation. Its placement,
parameters, credentials, and current global settings remain useful while the process is stopped and are exactly what a
crash successor needs. Registration should therefore follow configuration lifetime, while a Wayland connection follows
child lifetime.

## Goals

- Keep child ownership, reaping, restart backoff, registry-triggered retries, and build observation in `WidgetManager`.
- Keep a compositor registration across crashes, upgrade pauses, and ordinary restarts.
- Identify a widget connection with the configured widget instance ID instead of PID or per-spawn identity.
- Apply a registration before its first child can connect, so unknown connections are rejected rather than buffered.
- Close the exact widget connection promptly on temporary stop or removal without discarding retained configuration.
- Keep `ConfigHandle` and the secret store authoritative while ensuring the compositor's retained record is current.
- Preserve live parameter, credential, geometry, and global-setting updates.
- Prevent concurrent lifecycle operations from creating overlapping children for one instance.
- Remove the manager-to-compositor lifecycle event adapter and compositor-side process supervision.

## Non-goals

- Change restart delays, earned backoff rungs, registry refresh policy, or build observation policy.
- Change public gRPC, protobuf, or frontend APIs.
- Treat the widget key as a security boundary against a malicious root process. All native widget components run as root
  and can already inspect or control peers.
- Keep a PID-only compatibility path for old widget clients.
- Preserve spawn-first replacement. The simpler identity model deliberately uses stop-before-start.

## Core model

The three relevant lifetimes are independent:

- compositor registration: from configured-widget creation until configured-widget deletion;
- Wayland connection: from one child connection until that connection closes or is explicitly disconnected;
- managed child: from one successful spawn until exit and reap.

The stable `WidgetInstanceKey` is the configured widget's instance UUID (`Widget.id`). It is not the manifest
widget-type UUID: several configured instances may use the same type. A registration and every connection for that
logical instance use the same key across restarts.

The following invariants define the design:

1. A registration is applied before the manager is asked to perform the first spawn.
2. At most one child for an instance may be running or stopping. A successor is not spawned until its predecessor has
   been reaped.
3. At most one Wayland client is attached to a registration. A new client for an `Accepting` record atomically closes
   the old exact client before becoming the current attachment. Strict child non-overlap and the thin-host EOF contract
   make this the successor cleanup path, not process-incarnation arbitration.
4. A connection for an unknown or inactive key is rejected and closed. It is never retained for later registration.
5. Unregister and deactivate close the exact attached `ClientId` before acknowledging completion.
6. A thin control connection is the lifetime witness for the transferred Wayland connection. Control EOF is handled
   before ready Wayland events for that slot and closes the slot's exact Wayland connection.

These invariants make PID reuse irrelevant. PID remains diagnostic process information in the manager and logs, but is
not compositor identity.

## Ownership

### Widget manager

`WidgetManager` is only a process supervisor. Its actor owns running children, children undergoing graceful stop,
pending restarts, restart tokens, prepared-start permits, backoff state, build identity, launch records, and the widget
registry reference. It does not own compositor or configuration handles and emits no compositor lifecycle events.

Each supervised instance retains the data required for automatic restart and build observation:

```rust
struct WidgetLaunch {
    config_key: WidgetConfigKey,
    env: WidgetEnv,
}
```

`WidgetConfigKey` contains scene ID, widget instance ID, and expected widget-type ID. `WidgetEnv` contains only the
Wayland display location. The manager uses the type ID to find the executable. It passes the instance ID separately to
`WaylandSpawner`, which serializes it into `BMC_WIDGET_KEY` for every spawn.

Manager mode is an enum:

```rust
enum ManagerMode {
    Running,
    Paused,
    ShuttingDown,
}
```

`Running` accepts starts and retry acceleration. `Paused` rejects them during a firmware upgrade. `ShuttingDown` is
terminal and rejects every operation that could create a child, including commands sent through manager handles held by
detached work.

The actor also owns one prepared-start permit per instance to bridge source validation and a later spawn after
compositor receipts. Preparing a start succeeds only in its expected manager mode and replaces any earlier permit for
that instance. Any `Stop` for the instance invalidates its permit, including a stop with no supervised child; `Pause`
and `Shutdown` clear every permit. `Resume` preserves permits prepared while `Paused`, so upgrade resume can validate
and prepare under paused source locks, enter `Running`, and use those same permits. `Spawn` atomically removes and
matches the permit before checking other prerequisites. A missing or replaced permit returns `Superseded`, and the old
caller does not retry. The permit is internal in-flight synchronization, not PID, child-incarnation, generation, stable
protocol identity, or a security credential; it never enters compositor state or the child environment.

The actor assigns an internal token to each restart timer and accepts a timer only while that token still names current
manager state. A child task reports exactly one exit by instance key. Because Start rejects an instance while its child
is running or stopping, and stopping is removed only when the actor processes that exit, no successor can exist before
the predecessor's exit is consumed; a separate child-incarnation token is unnecessary. Exit handling schedules a restart
only in `Running`; an exit received in `Paused` or `ShuttingDown` removes the finished child without creating pending
state. `RestartDue` rechecks both its token and current mode immediately before spawn and becomes inert outside
`Running`. The restart token never leaves the manager and never enters compositor state.

Each instance occupies one manager state: running, stopping, or pending restart. A stop cancels pending restart or moves
a running child to stopping until its exit is observed; no separate stopping collection is needed because a successor
cannot overlap it. The stop request returns a termination handle so callers can enqueue it under an authoritative
configuration lock, release the lock, and then await the reap. A start received while that instance is running or
stopping does not replace it. It returns an explicit occupied result; callers revalidate and treat an already-running
instance with current launch data as satisfied.

A `Start` received for `PendingRestart` cancels the armed timer and attempts an immediate spawn with the command's
current launch data. It preserves the earned backoff rung if that attempt also fails, so an operator or upgrade retry
cannot create a second timer or reset a crash loop. Success replaces the pending entry with one running child.

If a guarded `Start` reaches an installed widget type but process spawn fails, the actor retains its `WidgetLaunch` as a
`PendingRestart` at the initial backoff rung before returning the spawn error. Later attempts use the existing timer,
registry recheck, and climbed-backoff behavior. An explicit start therefore receives automatic transient-failure
recovery without a registry-reload event. Stop, deletion, pause, and shutdown cancel that pending state normally.

### Coordinator

The coordinator owns the relationship between configured instances, retained compositor registrations, and manager
commands. There is no registration adapter or manager event stream.

Every lifecycle operation follows source-of-truth ordering:

1. acquire the configuration lock and, when needed, the secret-store lock;
2. validate the complete `WidgetConfigKey` and construct the current registration or update;
3. enqueue the compositor command while the authoritative lock is still held;
4. enqueue any manager stop/start command that must be ordered against the same configuration state;
5. release the locks before awaiting compositor application or child termination;
6. revalidate configuration after a wait before activating or starting an instance.

Every ordinary activation/start flow also reads the manager's current mode and prepares an expected-`Running` start
permit under its validating configuration lock. `WidgetManager` exposes the mode synchronously from actor-published
state, so this check does not wait while holding the lock. After compositor receipts, the flow revalidates under source
locks and sends `Spawn` with the same permit. Any intervening stop invalidates that in-flight start even if the caller
crossed the compositor cutoff later. Upgrade resume is the sole exception allowed to prepare and enqueue activation
while `Paused`; terminal `ShuttingDown` is never bypassed.

The compositor FIFO orders registration and hot updates. The manager actor orders process operations. Revalidation
bridges waits between the two without holding application locks across blocking compositor work or graceful shutdown.

### Compositor

The compositor owns one retained record per configured instance:

```rust
struct WidgetRegistration {
    key: WidgetInstanceKey,
    connection_mode: WidgetConnectionMode,
    placement: WidgetPlacement,
    initial: WidgetInitialConfig,
    attachment: Option<WidgetAttachment>,
}

enum WidgetConnectionMode {
    Accepting,
    Inactive,
}

struct WidgetAttachment {
    client_id: ClientId,
    protocol_surface: DeckWidgetSurface,
    render_state: WidgetRenderState,
}
```

The retained record contains current geometry, parameters, resolved credentials, and other initial configuration even
when `attachment` is absent. Protocol objects, render surfaces, frame callbacks, buffers, and client-originated actions
belong to the exact attachment, never to a PID.

## Compositor API and ordering receipts

The lifecycle-facing API becomes:

```rust
fn enqueue_register_widget(
    &self,
    registration: WidgetRegistration,
) -> Result<CompositorReceipt, CompositorError>;

fn enqueue_activate_widget(
    &self,
    key: WidgetInstanceKey,
) -> Result<CompositorReceipt, CompositorError>;

fn enqueue_deactivate_widget(
    &self,
    key: WidgetInstanceKey,
) -> Result<CompositorReceipt, CompositorError>;

fn enqueue_unregister_widget(
    &self,
    key: WidgetInstanceKey,
) -> Result<CompositorReceipt, CompositorError>;
```

Enqueue occurs synchronously and establishes FIFO order. The receipt is awaited only after configuration and secret
locks are released. Registration and activation receipts are required before spawn, because a fast client must never
arrive before the corresponding active record is applied. Deactivation and unregister receipts are required before the
caller considers the widget connection closed or permits a successor start.

Every receipt retains the existing two-second `WIDGET_COMMAND_ACK_TIMEOUT`. Register or activate timeout is a hard
pre-spawn failure. Deactivate or unregister timeout is reported and cannot be claimed as a completed connection cutoff,
but it never prevents the corresponding manager stop/reap: the command remains FIFO-enqueued, and child exit also closes
the connection. A wedged compositor therefore cannot keep children alive during ordinary stop, replacement, upgrade, or
terminal shutdown.

Hot parameter, geometry, and global-setting commands remain enqueue-only. Credential updates also enqueue under their
source locks, but return a bounded receipt reporting whether the compositor's retained credentials changed semantically.
The receipt is awaited only after releasing source locks. The registration/update ordering has two safe outcomes:

- If registration enqueues first, the later update changes the retained record before or after the child attaches.
- If the update wins the source lock first, registration reads the updated authoritative value. An update sent before
  the record exists may be a no-op because the later registration carries that value.

Registering an existing key updates its retained configuration. It does not disconnect an attachment unless the
operation explicitly includes deactivation. Lifecycle callers use explicit deactivate, stop, update/register, activate,
start ordering for changes that require a process restart.

Deactivation atomically changes the record from `Accepting` to `Inactive` before it captures and closes the exact client
and removes attachment-owned protocol, render, callback, buffer, and lifecycle-emitter state. It retains placement and
initial configuration. A repeated deactivation is idempotent. Activation changes an existing `Inactive` record to
`Accepting`; activating an already-`Accepting` record is an idempotent no-op that does not detach its client. Activation
cannot create a record. Detach after ordinary client loss also forgets per-attachment lifecycle-emitter state so a
successor receives an initial lifecycle event derived from current scene state.

Unregister removes the entire record and closes its exact attached client. A repeated unregister is idempotent. A later
request carrying that key is rejected unless a new registration has already been applied.

## Widget protocol

The production compositor exposes `deck_widget_manager_v2`, whose factory requires a widget instance key when creating a
widget surface. It continues to create the unchanged `deck_widget_surface_v1`; only manager admission changes.
Conceptually:

```text
get_widget_surface(widget_instance_key, wl_surface) -> deck_widget_surface_v1
```

This is an intentionally breaking native protocol change. The final compositor advertises only `deck_widget_manager_v2`,
and bundled clients require it; there is no `deck_widget_manager_v1` fallback or support for older widget clients.
Implementation may advertise both manager globals for one compile-safe transition commit, but the atomic cutover stops
advertising v1 and the following cleanup deletes it.

The stable key is transported as the canonical widget-instance UUID string and parsed with the repository's existing
UUID type. `deck_widget_surface_v1` events and requests do not otherwise change. Production does not advertise the old
PID-identified factory, and there is no request that omits the key.

Direct widget clients read `BMC_WIDGET_KEY` from their launch environment. The thin reads the same value and includes it
in the versioned `HelloMsg::Load` message beside the transferred Wayland fd. The shared host uses it only to create that
slot's widget surface; guest WASM does not receive the key. Missing or malformed launch values fail the client before it
creates a surface, while malformed thin control messages reject only that slot.

The key is routing identity, not authentication or authorization. The compositor trusts native processes that can reach
its Wayland socket; on the deployed system those components run as root and can already inspect or control one another.
The redesign adds no runtime-directory permission checks or secret-token handling. The shared host remains responsible
for keeping the key outside guest WASM, preserving the existing guest/native boundary.

When the compositor receives the factory request:

- an `Accepting` registration with no attachment accepts it and binds that exact `ClientId`;
- an unknown or inactive key is a protocol error and the exact client is closed;
- a key with another attached client closes that old exact client and installs the newcomer atomically;
- the same attached client may recreate its protocol surface after destroying the prior surface; a different client
  takes over only through the atomic close-and-replace rule above.

Every widget-originated request is accepted only from the registration's current protocol surface and `ClientId`. Object
destruction detaches only when both still match, so stale destruction cannot remove a later attachment.

A direct predecessor can leave a factory request buffered before it exits. If the compositor dispatches a healthy
successor first and that dead predecessor request second, the close-and-replace rule can evict the successor once. The
predecessor then detaches on EOF and the manager restarts the successor through normal backoff. This is an accepted
self-healing consequence of using a stable routing key without process-incarnation identity: it can cause one delayed
reattach but cannot disclose another widget instance's configuration or leave permanent blank state. The shared-host
control-first contract prevents the analogous hosted-widget ordering.

There is no pending-connection buffer. Registration-before-spawn is the only supported ordering, and accepting an
unknown key for later would weaken the invariant that makes a stable key sufficient.

## Shared-host handshake

The shared host must preserve the thin as the lifetime witness for its transferred Wayland connection. On every loop
iteration it processes ready control sockets for all existing pending and active slots before accepting or advancing a
new slot and before dispatching any Wayland events. EOF or hangup drops the slot and closes its exact Wayland connection
before that slot can submit another request or commit. A successor therefore cannot issue its factory request while a
dead predecessor slot still awaits ready control EOF in the same host.

Accepting a new thin must not block established slots while the new slot waits for initial configuration. The current
synchronous `WidgetSlot::from_handshake` path becomes a pending slot polled by the main loop with the existing configure
deadline. Existing slots continue dispatching, rendering, and processing control sockets. Only a completed initial
configure constructs the runtime and promotes the pending slot. Pending slots count toward host liveness and the
pre-exit backlog sweep, so the host cannot exit while a successor is still configuring. No thread or cross-slot global
timeout is added. Promotion records a load in `HostLifetime`. Any deferred configuration, key, module, or load failure
records a rejection with the same accounting as a synchronous `accept_and_load` failure, so removing the final pending
slot can make the host eligible to exit. Dropping a pending slot because its thin control socket reaches EOF or hangup
records the same rejection.

## Lifecycle flows

### Initial creation and service startup

For each configured instance:

1. Read and validate its complete configuration key, placement, installed manifest, and credential bindings under the
   normal configuration-before-secrets lock order.
2. Enqueue a retained registration containing current initial configuration. A new record starts `Inactive`; an existing
   record preserves its connection mode.
3. For an instance currently shown by enablement or active preview in manager `Running` mode, enqueue activation under
   the same source snapshot. The compositor FIFO applies registration before activation.
4. Release the locks and await the registration and, when present, activation receipts.
5. Reacquire the preview and configuration locks, revalidate the same type, placement, shown predicate, registry build,
   and `Running` mode, then complete the manager `Start` command send.
6. Release the locks before awaiting the spawn reply.

If configuration changes before activation or Start, the operation follows current state rather than spawning its stale
snapshot. If registration application fails, no child is started. A concurrent later configuration operation may update
or remove the record through normal FIFO ordering.

### Unexpected exit and restart

The child task reports one exit by instance key. The manager removes the corresponding running or stopping state before
it admits any successor; an unexpected running-child exit records the backoff rung and pending restart. The closed
direct socket, or thin control EOF in the shared host, detaches the compositor attachment. The retained active
registration and its current configuration remain.

When the timer expires, the manager rechecks the installed registry entry and starts the successor with the same stable
key. The connection attaches to the existing active registration and receives the latest stored initial batch. No
compositor lifecycle event, registration update, PID bind, or pending connection is involved.

An explicit stop or deletion racing the timer is ordered by the manager actor. If restart wins first, the following stop
terminates that child; if stop wins first, the stale timer token cannot spawn. Pause and terminal shutdown likewise make
every queued timer inert. Child exits caused by either mode transition remove the finished child without arming a
replacement timer.

### Temporary stop and resume

Temporary stop retains configuration:

1. Enqueue deactivation and complete the manager `Stop` send under the validating configuration lock.
2. Release the lock and await both the receipt and termination handle. Report deactivation timeout without abandoning
   child stop/reap.

Configuration reconciliation keeps the retained record current independently of process lifecycle. Resume revalidates
authoritative configuration and current shown eligibility, prepares an expected-`Running` start permit, then enqueues
only activation under the source locks. After awaiting its receipt, it reacquires the source locks, revalidates
eligibility and `Running` mode again, and sends `Spawn` with the same permit. It releases the locks before awaiting the
reply. A spawn failure leaves an active, detached registration. The manager retains a pending restart and retries with
ordinary backoff; `Superseded` instead ends the old caller because a newer lifecycle operation owns the instance.

### Restart-requiring update

A geometry, widget-type, or build change uses stop-before-start:

1. Commit the authoritative change, enqueue deactivation plus the retained-record update, and complete the manager
   `Stop` send under the source lock.
2. Release locks and await both deactivation and child termination. Report deactivation timeout without abandoning
   stop/reap.
3. Reacquire the source locks and revalidate the complete configuration key, shown eligibility, and `Running` manager
   mode.
4. Enqueue any newer retained-record update followed by activation.
5. Release locks and await activation.
6. Reacquire the configuration lock, revalidate configuration and `Running` mode again, complete the manager `Start`
   send under that lock, release it, and await the reply.

If another operation already started the current launch while this task waited, the manager's occupied result is treated
as satisfied only after confirming that the running launch matches current configuration. If it does not match, the
caller performs one bounded retry of deactivate, stop, revalidate, activate, and start. A second mismatch is reported as
an invariant failure rather than looping. No operation spawns over a running or stopping predecessor. A failed successor
spawn leaves the cell blank; preserving the predecessor is intentionally not part of this simpler contract.

### Deletion

Configured-widget deletion enqueues unregister and completes an unconditional manager `Stop` command send under the
configuration write lock. After releasing the lock, it awaits both the unregister receipt and the termination handle.
The FIFO relationship between every configuration-dependent start and stop send means a start validated before deletion
is followed by this stop, while a start attempting validation afterward sees no configuration. Once unregister applies,
reconnects using the deleted key are rejected even while graceful process termination is still underway.

### Firmware upgrade

`stop_all_widgets` first moves the manager to `Paused` and awaits that transition, removing every pending-restart entry
and preventing starts without yet signalling current children. Exit reports after that transition cannot recreate
pending state. It then acquires the configuration write lock, enqueues deactivation for all retained registrations, and
releases the lock before awaiting receipts. It stops and reaps every current or already-stopping child even when a
deactivation times out. A timeout is reported to the upgrade operation; a wedged compositor must not leave widget
processes running. Registrations retain current configuration throughout the upgrade.

`restart_widgets` takes the preview and configuration read locks and selects configured instances currently shown by
enablement or active preview. For each instance it prepares a start permit scoped to `Paused`, activates the retained
record, and releases the locks before awaiting those receipts. It does not register or update records; configuration
reconciliation owns that work throughout the pause. It then reacquires the same source locks, revalidates the complete
shown set and `Paused` mode, and moves the manager to `Running`. Resume preserves the paused permits, and each `Spawn`
atomically consumes its permit before the locks are released and replies are awaited. A failed or cancelled upgrade uses
the same guarded resume path. If terminal shutdown won the race, `ShuttingDown` clears the permits, absorbs resume, and
no child starts.

### Terminal shutdown

Startup owns shutdown ordering. It first stops gRPC and other mutation producers and aborts and joins the
registry-reload task. A short manager command then enters `ShuttingDown`, removes every pending-restart entry, and makes
later exit reports unable to arm replacement timers without yet signalling current children. Retained manager handles
immediately receive the terminal result and cannot create children. Startup deactivates retained registrations and
awaits every receipt so clients cannot commit or reconnect during graceful termination. It enqueues those deactivations
under the configuration write lock after the `ShuttingDown` transition, releases the lock, and only then awaits the
receipts. An upgrade resume that already holds the lock enqueues activation before the terminal deactivations; one that
acquires it afterward observes `ShuttingDown` and cannot activate. Startup then asks the manager to stop and reap every
current or already-stopping child. The actor remains responsive to internal exit reports while the terminal waiter is
outstanding.

After all children are reaped, startup shuts down the compositor, which drops all retained registrations. There is no
lifecycle channel to close, adapter to drain, or final per-widget unregister pass.

## Parameter and credential updates

Size-preserving parameter updates remain live:

1. Acquire the configuration write lock.
2. Validate and save the new parameters.
3. Enqueue `UpdateWidgetParams` against the retained record.
4. Release the lock.
5. Accelerate a pending restart after a successful enqueue.

The retained record exists while the child is stopped, so the update is not lost and the next attachment receives it. A
live surface receives the same update immediately. A send failure is reported and leaves restart timing unchanged; saved
configuration remains authoritative.

Credential refresh remains a broad background listener. It subscribes to scene and account changes because either can
change a widget's resolved bindings. After a notification it coalesces queued notifications. It acquires configuration
then secret-store read locks and re-resolves every distinct configured widget from one consistent snapshot. It enqueues
an `UpdateWidgetCredentials` command for every resolvable retained registration, releases both locks, and awaits the
bounded receipts concurrently.

Account mutation runs in a detached persistence task, so request cancellation cannot interrupt an accepted save or
delete. A successful non-idempotent save publishes the account notification consumed by the independent listener. An
idempotent account update returns before saving and emits no notification. Scene persistence publishes the corresponding
scene notification. No handler computes an affected-widget subset or directly owns credential delivery.

A missing installed manifest is reported. Initial registration withholds bound credentials it cannot authorize. A hot
refresh preserves the retained last authorized values for that instance rather than replacing them from unchecked
bindings. When the manifest becomes available, normal registry reload re-resolves and updates the retained record before
starting it.

Each credential command overwrites retained initial credentials and re-emits them to a current attachment only when the
resolved view or secrets differ. The compositor reports that semantic result in the receipt. The listener accelerates
only the corresponding changed pending widgets; unchanged broad fan-out does not disturb their earned backoff. A send or
receipt failure is reported without mutating manager state. No affected-set cache, invalidation worker, or credential
retry queue is added.

## Global settings

Timezone, localization, night mode, brightness, volume, and similar broadcasts remain unchanged. The compositor's
current-setting cache supplies them to each attachment's initial configure batch. Retained widget records need not copy
values that are already global compositor state.

## Build observation

The manager-owned `WidgetLaunch` replaces the coordinator's duplicate spawn-record map. A snapshot query exposes
supervised instance IDs, complete configuration keys, and current build identities. Running, pending-restart, and
stopping instances all count as supervised. A stopping child retains enough launch and build identity for reload to
avoid misclassifying an in-progress replacement as a missing instance.

Registry reload enumerates every configured instance whose type is currently installed, rather than only instances the
manager still supervises. A missing instance goes through registration reconciliation regardless of whether its scene is
shown or the manager is paused. Matching supervised instances already have current build-owned registration data and are
left alone; an in-progress stop retains ownership of its own replacement. Any missing shown instance in `Running` mode
continues through guarded activation and Start. This recovers both a pending restart that ended while its type was
absent and a guarded start that never reached the manager because its compositor prerequisite failed.

For a supervised instance whose build changed, reload deactivates the registration and requests Stop under the
validating configuration lock. It releases the lock while awaiting cutoff and termination, then reconciles current
configuration membership again. This installs current retained data even when a preview ended or process lifecycle
became paused while the cutoff was pending. Only a still-shown instance in `Running` mode proceeds through activation,
its receipt, another configuration and mode recheck, and the manager `Start` send. A concurrent deletion, disable,
preview teardown, pause, or widget-type change therefore cannot be resurrected from the original snapshot.

Pending restarts remain in the manager snapshot but are not separately restarted by build observation; their timer uses
the refreshed registry entry. If upgrade pause or terminal shutdown wins while reload awaits termination, the final
start is rejected and the authorized resume path or reboot owns recovery.

## Failure handling

- Registration or activation enqueue/application failure prevents spawn and is returned with context. This is a
  compositor service fault rather than a child-process failure; it is not converted into a manager restart because the
  registration-before-spawn prerequisite is unproven. A later configuration operation, registry reload, or service
  restart reconciles it.
- A missing or malformed `BMC_WIDGET_KEY` prevents the client from creating a widget surface.
- An unknown, inactive, or malformed key is a protocol error that closes only the offending exact client. A valid key on
  an `Accepting` record atomically replaces any stale exact attachment.
- A thin control-message or slot-load failure rejects that slot and causes its thin to exit; ordinary manager backoff
  applies.
- A missing widget type ends an automatic pending restart. Registry refresh compares current shown configuration with
  the manager snapshot and starts the missing instance when the type becomes available; the retained registration
  remains available in the meantime.
- A failed explicit guarded process spawn retains a manager pending restart and retries without registry activity. The
  reload diff remains a backstop for configured instances absent because start never reached the manager.
- Compositor hot-update send failure is reported but does not mutate manager state or backoff.
- Manager `Paused`, `ShuttingDown`, occupied, and spawn-failure results are distinct and handled according to the flows
  above.

## Removed mechanisms

The redesign removes:

- coordinator-owned `WidgetGeneration` and generation-stamped compositor commands;
- manager lifecycle events and the registration adapter task;
- `set_widget_pid`, `clear_pid`, `bind_respawned_pid`, and `unregister_abandoned`;
- compositor PID fields and `SO_PEERCRED` identity lookup;
- pidfds, PID tombstones, stale-bind guards, and PID-based purge paths;
- the pending Wayland connection buffer and its capacity/eviction policy;
- registration churn on crash and respawn;
- spawn-first replacement and overlapping predecessor/successor stopping state for one instance.

The manager keeps only internal restart tokens for queued timers and prepared-start permits for in-flight coordinator
work. Neither is process or protocol identity.

## Verification

### Widget manager

- Verify one actor exclusively owns every child and every wait/reap operation.
- Verify unexpected exit preserves the backoff rung and respawns with the same instance key without a lifecycle event.
- Verify a successor cannot start until the predecessor exit is processed, and stale restart tokens cannot spawn.
- Verify start rejects a running or stopping instance and never overlaps two children for one key.
- Verify preparing a start is scoped to the expected mode, a newer prepare or any Stop supersedes the old permit, and
  Spawn consumes a matching permit exactly once.
- Verify pause and shutdown clear prepared starts, while resume preserves permits prepared in `Paused` for one atomic
  post-resume Spawn.
- Put an instance in `PendingRestart`, issue explicit `Start` with current launch data, and verify it cancels the timer,
  attempts immediately, and preserves the earned rung if that attempt fails.
- Verify stop cancels pending restart, waits for reap, and repeated stop joins the same stopping child.
- Verify pending, unknown, and empty stop operations resolve without waiting for an exit that cannot occur.
- Verify `stop_all` enters `Paused`, cancels timers, waits for current and already-stopping children, and rejects starts
  and retry acceleration until explicit resume.
- Verify terminal shutdown enters `ShuttingDown` before stopping children and retained handles cannot resume or start.
- Race a child exit and queued `RestartDue` against both pause and shutdown; verify neither creates pending state or a
  successor and both mode transitions drain every pre-existing pending entry.
- Verify registry refresh accelerates eligible pending restarts without resetting their earned rung.
- Verify a missing type ends the pending attempt without touching compositor state.
- Restore a previously missing type and verify registry refresh discovers the enabled instance absent from the manager
  snapshot and starts it through the guarded flow.
- Fail an explicit guarded process spawn with an installed type and verify manager backoff retries it without registry
  activity.
- Fail registration application before Start reaches the manager, trigger an unrelated registry reload, and verify the
  configured-but-unsupervised instance is retried.
- Preserve build-observation tests, including re-observation and revalidation across graceful stop.
- Include a stopping instance in the manager snapshot and verify registry reload does not treat it as missing during
  replacement.

### Coordinator and ordering

- Verify registration is enqueued under source locks, its receipt is awaited after releasing them, and no first spawn
  occurs before application.
- Deterministically park an ordinary start after its permit and activation are enqueued, cross the cutoff with a Stop,
  then release the compositor receipts; verify the old Spawn returns `Superseded`, does not retry, and creates no child.
- Gate registration and parameter-update lock acquisition in both orders and verify the retained value is current.
- Repeat the ordering test for account changes and credential resolution.
- Verify initial registration with a missing manifest withholds credentials and does not spawn an unavailable type.
- Verify deactivation is applied before temporary stop or restart-requiring replacement can proceed.
- Verify configuration is revalidated after every termination wait before activation or start.
- Reproduce concurrent restart-requiring updates and prove no overlapping child starts and final state matches current
  configuration.
- Park resume between activation and its lock-ordered start send, delete the instance, and verify deletion's
  unconditional stop follows any earlier start while no later start can validate.
- Return occupied with a mismatched running launch and verify one bounded stop/start retry converges; repeat the
  mismatch and verify it fails without looping.
- Verify deletion unregisters before graceful child termination completes and the key cannot reconnect.
- Verify upgrade pause retains registrations but closes connections, while resume activates and reuses them without
  registering or updating them.
- Signal registry reload after upgrade pause and verify retained configuration updates may apply while every
  registration remains `Inactive` until the authorized resume path activates it.
- Verify shutdown stops mutation producers, joins registry reload, deactivates clients, terminally stops the manager,
  and then shuts down the compositor without an adapter task.
- Park upgrade resume after its `Paused` mode check while it holds the configuration lock, enter terminal shutdown, and
  verify terminal deactivation follows any activation it can enqueue and no registration remains `Accepting` after the
  shutdown receipts complete.
- Wedge compositor receipt delivery and verify the existing timeout prevents initial spawn and cannot keep upgrade or
  terminal child shutdown from running.
- Wedge deactivation receipt delivery during temporary stop and restart-requiring replacement; verify the fault is
  reported while every targeted child is still stopped and reaped.

### Hot updates

- Verify parameter updates persist while detached and are included in the next initial batch.
- Verify any scene or non-idempotent account notification refreshes every distinct configured retained registration,
  including detached records.
- Verify an idempotent account save emits no refresh. Broad semantically unchanged updates must not accelerate a pending
  widget.
- Verify compositor semantic-change replies accelerate exactly the pending widgets whose retained credentials changed.
- Verify configuration-before-secrets ordering for concurrent account update and deletion.
- Verify source locks are released before credential receipts are awaited.
- Verify account deletion refreshes the full configured set after unbinding.
- Verify request cancellation cannot interrupt the detached account mutation or the independent listener refresh.
- Verify compositor send and receipt failures leave manager state and timers unchanged.

### Protocol and compositor

- Verify production exposes only the key-bearing factory and bundled direct and hosted clients require the launch key.
- Verify an active registered key attaches and receives current geometry, parameters, credentials, and global settings.
- Verify unknown, inactive, and malformed keys close only the offending exact client and are never buffered.
- Leave a dead direct client's attachment uncleared, connect its legitimate successor, and verify the newcomer closes
  the old exact client and becomes the sole current attachment.
- Dispatch a dead direct predecessor's buffered factory request after its healthy successor attaches and verify the one
  spurious eviction detaches on predecessor EOF and manager backoff restores the successor without permanent blank state
  or cross-instance configuration delivery.
- Verify multiple configured instances of one widget type remain distinct because they use instance IDs, not type IDs.
- Reuse a numeric PID and verify it has no effect on registration or attachment selection.
- Verify render surfaces, frame callbacks, actions, and cleanup are attributed by exact `ClientId` and protocol-surface
  identity, never PID.
- Verify deactivation closes the exact client and clears attachment-owned state while retaining initial configuration.
- Verify detach and deactivation forget lifecycle-emitter history so a successor receives an initial current-state
  lifecycle event.
- Verify activation permits a later connection with the same stable key.
- Verify activation of an already-`Accepting` record is an idempotent no-op that preserves its attachment.
- Verify unregister closes the exact client, destroys callbacks and buffers, and removes the retained record.
- Verify stale object destruction cannot detach a later connection and detach/unregister emit one disconnected
  notification per attached interval.
- Remove pending-buffer, generation, tombstone, guarded-bind, and abandonment tests with their implementation.

### Shared host

- Verify thin control EOF tears down the corresponding pending or active slot and exact Wayland connection before any
  ready Wayland event for that slot is dispatched.
- Make predecessor control EOF and listener accept ready together and verify all existing control sockets are processed
  before the successor can issue its factory request.
- Hold one slot before initial configure and verify established siblings continue dispatching, rendering, and processing
  control sockets within their normal deadlines.
- Remove the last active slot while a pending slot configures and verify the pending slot keeps the host alive and is
  included in the pre-exit backlog sweep.
- Complete the only pending slot successfully and verify it records a host load. Fail it after deferral and verify it
  records a rejection and permits host exit when no slots or overlays remain. Repeat the failure by closing its thin
  control socket before initial configuration completes.
- Verify the thin forwards the key only to its slot and guest WASM does not receive it.
- Verify a rejected key fails one slot without affecting siblings.

Run focused manager, coordinator, scene-management, account-management, compositor, widget-client, thin-protocol, and
shared-host tests first. Run `just validate` after the focused suites pass.

## Expected implementation surface

The change is expected to touch the widget manager and coordinator, compositor trait and test double, startup and
shutdown wiring, scene and account handlers, secret-store refresh wiring, mock compositor, OpenWRT compositor commands
and protocol state, `bmc-widget-protocol`, bundled widget clients, thin-host control protocol, and shared-host slot/main
loop. It also updates `docs/devel/widget-lifecycle.md`, `docs/devel/widget-runtime-configuration.md`, and the WASM-host
process-model documentation.

It requires no protobuf, frontend, pidfd, or Rustix process/event feature changes. Developer documentation records the
stable instance-key trust model, registration-before-spawn invariant, stop-before-start replacement, exact-client
cleanup, and nonblocking shared-host handshake.
