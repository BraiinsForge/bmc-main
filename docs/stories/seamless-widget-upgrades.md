# Seamless Widget Upgrades

Widget upgrades apply through the device's package profile without restarting the whole display when the compositor
itself is unchanged. The running scene remains visible while each changed widget instance is replaced with the newly
activated package.

## User stories

### Upgrade a widget without interrupting the display

> As a user, I want widget updates to appear without the screen going blank so an ordinary widget release does not
> interrupt the rest of my dashboard.

- Activation asks the running compositor to refresh its widget registry.
- Unchanged widget instances keep running.
- Each running widget instance whose package changed is stopped and spawned from the activated profile; newly added
  widgets become available in the picker.
- Removing a widget package removes it from the picker; running instances keep running until their scene next changes,
  and one that crashes after the removal is not brought back — its cell stays empty.

### Upgrade the widget runtime safely

> As a user, I want runtime upgrades to replace incompatible background processes automatically so widgets recover
> without manual intervention.

- The compositor starts and supervises the host for the active SDK major independently of widget startup.
- Widget thins wait for that host and never choose, start, or replace it.
- A graceful compositor restart stops its host before the new compositor starts the replacement.
- After forced compositor termination, socket arbitration rejects a replacement while the old host is alive; the
  supervisor retries with backoff until it can bind.

### Restart the display only when its executable dependencies change

> As a user, I want the display to restart only when the compositor or its native widget runtime actually changed.

- The compositor service records the per-SDK thin launcher and host as independent executable dependencies.
- A compositor, thin, or host package change alters those dependencies and lets the service orchestrator restart the
  compositor and its host.
- Widget package changes do not alter the core package or compositor service, so they use targeted replacement instead.
- The deployment tool observes activation but never issues a compatibility hard restart of the compositor.

## Failure behavior

- A missing widget root empties the registry; an unreadable root or failed scan keeps the last valid registry. An
  invalid manifest is logged and that one widget skipped.
- A widget that fails to come back is retried with a widening delay, and never takes down other widget instances. It
  stays stopped only when its type is no longer in the registry, and then its grid cell stays empty.
- Deployment fails loudly when the compositor is down, the service reconciliation does not settle, or the active core
  predates targeted widget reload; it reports the observed service state and leaves widget load verification to the
  device validation procedure.
- A crashed compositor is respawned by the service manager, and the new compositor starts a new host before thins
  reconnect.

## Constraints

- Targeted replacement preserves the compositor and unaffected widgets, not the internal state of the widget instance
  being upgraded.
- Compositor or runtime dependency upgrades can still cause a brief full-display restart.
- Lifecycle serialization across simultaneous independent reload requests is outside this behavior and remains tracked
  separately.
