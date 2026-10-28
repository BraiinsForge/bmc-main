# Config Persistence

The running config and the config file are two things:

- the in-memory `ConfigHandle` is what the BMC application acts on: the compositor, the widget coordinator and every
  gRPC reader see it;
- the file under `/etc/bmc` is what survives a reboot.

A save copies the first to the second; it never defines the first.

## A failed save keeps the change

When `ConfigHandle::save` fails, the change stays in memory and nothing is rolled back. The caller reports the failure
and carries on with the changed state:

- a gRPC handler answers an error;
- a background path logs a warning.

The reason is the broken filesystem case. If the overlay is full or read-only, every save fails until someone fixes the
device.

- Rolling back on failure would pin the user to the last state that happened to reach the disk: no setting could be
  changed, no widget added, no params edited, for as long as the fault lasts.
- Keeping the change lets the device stay usable in the meantime. The cost is that the next boot comes up from the older
  file; the user is told the save failed, so that cost is not a surprise.

`save` is built on this: it broadcasts the changed sections to their subscribers *before* writing the file, so the
compositor and the widgets follow memory whatever the write does.

## What this asks of a write path

- Mutate the in-memory config, call `save`, surface the error. Do not undo the mutation when `save` fails, and do not
  defer the mutation until the write succeeded.
- A path that mutates in memory only on a successful save — or that restores the previous value after a failed one —
  goes against this policy, however tidy a transactional write looks. One such path among many non-transactional ones
  also leaves the device in a state no single policy describes.
- A background writer (a migration, a scheduler) follows the same rule: apply, try to save, log on failure. The write
  retries at the next natural occasion — the next user save, or the next boot gate — not through a retry loop of its
  own.

## Known deviation

Account removal (the `remove_account` handler in `bmc/src/web/grpc/account_management.rs`) saves a clone of the config
and assigns it back only when the write succeeds. That is the transactional write this policy rules out; it is wrong and
awaits a fix, with no ticket filed yet.
