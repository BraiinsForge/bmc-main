# Service Orchestrator

The service orchestrator (`bmc-nix-service-orchestrator`) reconciles OpenWrt services after a profile activation: it
stops services the new generation removed, starts services it added, runs upgrade actions for services whose init script
changed, and reconciles `/etc/rc.d` enable symlinks. The binary lives in `bmc-nix/src/bin/service_orchestrator.rs`; the
discovery, diffing, and planning logic lives in `bmc-nix/src/service_orchestrator/`.

## Why a Detached Transient Service

Service reconciliation cannot run inside the activation process tree. The services being restarted may include the very
process that triggered the activation (for example `bmc-openwrt` upgrading a generation that replaces `bmc-openwrt`
itself), so a synchronous restart from an activation script would kill its own ancestor mid-activation.

Instead, the core package's `090`-prefixed activation script (`start-service-orchestrator`, defined in
`nix/pkgs/core/default.nix`) registers the orchestrator as a one-shot transient procd service via
`ubus call service set` and returns immediately. procd then runs the orchestrator detached from the activation, as
service `bmc-nix-service-orchestrator`. The script first deletes any stale instance left over from a previous
activation, so at most one orchestrator run is registered at a time; a revert re-registers the instance under the same
name, replacing the failed generation's registration.

The registered command receives:

- `--old-generation` — `$PROFILE_OLD_GENERATION`; empty on first activation (the empty path is the "no previous
  generation" sentinel, making every new-generation service classify as new).
- `--new-generation` — `$PROFILE_NEW_GENERATION`.
- `--current-link` — the `current` symlink next to the new generation; its parent directory is the profile directory
  used for locking.
- `--instance-name` — the transient service name, so the orchestrator can delete its own registration when done.
- `--timeout-seconds` — a single deadline (currently 300 s) covering the whole run, most importantly the profile-lock
  wait.

The binary is wrapped in an `/bin/ash` script that unsets `LD_PRELOAD` before exec, because procd starts the service in
a context where `LD_PRELOAD` may be set.

## The Profile Lock Handshake

The orchestrator synchronizes with the activation that spawned it through the [profile lock](profiles.md#profile-lock).
The sequence is:

1. The activation sequence runs with the profile lock held — either by the Rust caller
   (`bmc_nix::profile::activate_profile` and its callers) or by the generated activation entrypoint itself when run
   standalone.
2. The `090` script registers the orchestrator while the lock is still held. procd spawns the orchestrator as a fresh
   process, so it does not inherit the holder's lock file descriptor.
3. The orchestrator discovers and diffs the old and new generations first — generations are immutable, so this needs no
   lock — then blocks in `profile::lock_profile_with_timeout` until the deadline.
4. The lock only becomes available once the entire activation sequence has finished, including a possible
   [revert to the old generation](upgrades.md#deferred-activation---next-boot). Acquiring it therefore means "activation
   is over, in whatever direction it went".
5. Holding the lock, the orchestrator verifies that `current` points at `--new-generation`. This is the success
   handshake: if activation failed and was reverted, or a newer activation superseded this one, `current` points
   elsewhere and the orchestrator exits with an error without touching any service. On a revert, the old generation's
   own entrypoint re-registered the orchestrator with the old/new roles swapped, so the instance that survives is the
   one that reconciles services toward the reverted (old) generation.
6. The lock stays held for the rest of the run, so a concurrent upgrade or activation cannot swap generations in the
   middle of service reconciliation.

If the deadline expires while waiting — for example because the activation wedged — the orchestrator fails without
acting on services and only cleans up its transient registration.

## Service Discovery and Diff

`discover_generation` reads, per generation:

- `etc/init.d/` — the service init scripts; the script contents are the identity used for diffing.
- `etc/rc.d/` — `S<prio><name>` and `K<prio><name>` links, parsed into start and stop priority maps.
- `etc/init.d.conf/<name>.json` — optional per-service `ServiceConfig`; missing files fall back to the defaults below.

`compare_generation_services` classifies each service name found in either generation as new, removed, upgraded (present
in both with different init script contents), or unchanged (byte-identical init scripts).

## Action Plan

Per-service actions come from `ServiceConfig` (`bmc-nix/src/service_orchestrator/config.rs`), with these defaults:

| Field               | Default                 | Runs for                                                                                           |
| ------------------- | ----------------------- | -------------------------------------------------------------------------------------------------- |
| `init`              | `["boot", "start"]`     | new services                                                                                       |
| `removed`           | `["stop", "disable"]`   | removed services                                                                                   |
| `upgrade`           | `["disable", "reload"]` | upgraded services, gated by `upgrade_if_status`                                                    |
| `always`            | `["enable"]`            | every service in the new generation, every activation                                              |
| `upgrade_if_status` | `running`               | gate: run upgrade actions only when the service status matches (`running`, `stopped`, or `always`) |

Upgraded candidates are gated by their live status, read via `/etc/init.d/<name> status` before planning.
`build_action_plan` orders the result: removed services run first at their old-generation `K` priority using the old
generation's init script path (the script may no longer exist in the live root); everything else runs at
`100 + S priority` against `/etc/init.d/<name>`. `always` actions are emitted last within their priority bucket, so
`upgrade = ["disable", "reload"]` can wipe stale `rc.d` entries (including ones at an old START priority) and
`always = ["enable"]` reinstalls the correct symlink afterwards. The `always`/`enable` pass is also what reconciles
`rc.d` symlinks for unchanged services on every activation.

Execution is best-effort per action: a failed action is logged and counted, but does not stop the remaining plan.

## Cleanup and Logging

Whether orchestration succeeded or failed, the orchestrator finishes by deleting its own transient procd service via
`ubus call service delete`, so the registration does not linger until the next activation's stale-instance cleanup.

The orchestrator logs to `/var/log/nix-orchestrator/nix-orchestrator.log` (DEBUG level, size-rotated with compressed
history) and mirrors INFO-level output to stderr, which procd captures.
