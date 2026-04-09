# Services management

The service management has to be done just right, to provide the
possibility for users to upgrade without rebooting. When a package is
upgraded, it might produce a new /etc/init.d (OpenWrt) service. And
this service should be restarted in order to finalize the upgrade.

## Services not managed by Nix

There will be two services for Nix management that are not managed by
Nix - they run completely irrespective of the /nix/store. They will
be shipped with the BOS firmware. They cannot refer to the /nix/store.

### bmc-nix-initializer

This starts the binary that initializes the store. It should check
that the store exists and that a profile has been successfully
activated on this boot. At the end of activation, a profile should
write a file /tmp/nix-activated. The initializer waits for this file.
If the store is not present or the profile is not activated for too
long, initialization of the store is performed. This binary also
supports creating an AP so that the user can connect to WiFi for this
operation.

When the store is present but no profile `current` symlink exists
(e.g., after extracting the initial tarball), the initializer
activates the latest profile generation using the `bmc-nix` library
directly. This is necessary because the tarball does not run
activation during build (activation scripts reference absolute system
paths that don't exist in the build sandbox). The initializer breaks
the chicken-and-egg problem: activating the initial profile installs
the `nix-activator` service, which handles activation on subsequent
boots.

This service will run only after the boot has finalized, only as a
regular service, not using `boot()` function. In case it detects the
store is initialized, it exits. The service does not show anything on
the screen until it would start initialization of the store.

### nix-factory-reset

In order to finalize a factory reset, there will have to a be a
service that will remove the /nix/store so that the bmc initializer
may perform (re)initialization. During factory reset, the U-Boot
environment is always cleaned out. Thanks to this the service can
check it and see that `nix_init` U-Boot variable is missing. When that
happens, it removes the /mnt/data/nix under /mnt/data.

This service has to run before any Nix services so that the store is
not in use yet. There will be a contract that no Nix service should
start before this one. This contract cannot be strictly enforced by
us, because there are infinitely many ways a package could introduce an
OpenWrt service without us knowing. Still, the factory reset service should
try to kill the applications running from /nix/store, if there are any.

This service will perform its operation in the `boot()` function and it has
to run as early as possible. On the other hand it should run only after
`/mnt/data` has been mounted (or mount it itself)

## Services managed by Nix

Any package can declare an OpenWrt service. There is a standard way
for all packages to do so. These services are collected into an
activation script that will allow for restarting the services on
upgrade. Since the activation scripts know the old and new
generations, they can assess when an upgrade of service has happened.

Each service may provide a JSON configuration file that describes how
the orchestrator should handle it during upgrades. When the file is
missing, defaults are assumed. The full format is described in the
"Service configuration format" section below.

Crucially, it is realized that the activation scripts themselves may
be ran by such OpenWrt service managed by Nix. Implying that a restart
done during the activation would potentially lead to killing the
activation script itself. This is why the restart of the services will
also be done by a service. This service will receive as arguments:
- The PID of the activation process (the main activation script, not the individual ones)
- The services to stop (no longer existing services)
- The services to restart
- The services to start (new services)
This is implemented through the `bmc-nix-service-orchestrator`.

Then, during activation, this service is started. It is the only
service that is (re)started during activation itself. After the
service sees that the activation process is finished, it performs the
restarts.

### Package services structure

Each package can provide OpenWrt services in `etc/init.d`. These
services will be automatically symlinked to `/etc/init.d` on the
system, so they will be picked up by OpenWrt on boot. They might also
be started before the boot through the `bmc-nix-service-orchestrator`.

Each service may optionally provide a configuration file at
`etc/init.d.conf/<service>.json` that controls upgrade behavior.
See the "Service configuration format" section for the full schema.

### Services diffing

An activation binary directly scans `etc/init.d/` in both the old and
new generations. No pre-collected manifest is needed — the directory
contents and per-service JSON configs are the source of truth.

Services of the same name are compared by their file contents. If the
contents are identical, the service is considered unchanged and is not
touched. If the contents differ, the service is treated as upgraded
and its `etc/init.d.conf/<service>.json` determines what happens.

Services present only in the new generation are new. Services present
only in the old generation have been removed.

Based on this diff and the per-service configuration, the orchestrator
determines which services to start, stop, or reload after activation
has completed successfully.

When the old generation path is empty (first-ever activation, no
`current` symlink exists yet), the orchestrator treats every service
in the new generation as new. This means all services get their `init`
actions executed, which is how services are started for the first
time after the initial profile installation.

### Service configuration format

Each service may provide `etc/init.d.conf/<service>.json`. When
missing, defaults are assumed. The full default configuration:

```json
{
  "init": ["boot", "start"],
  "removed": ["stop"],
  "upgrade": ["reload"],
  "reboot_required": false,
  "upgrade_if_status": "running"
}
```

#### `init`

An ordered list of `/etc/init.d/<service>` actions to run when the
service first appears (the old profile did not have it, the new one
does). Each entry is a standard OpenWrt init.d action such as `boot`,
`start`, `stop`, `restart`, `reload`, `enable`, or `disable`.

Default: `["boot", "start"]`.

#### `upgrade`

An ordered list of `/etc/init.d/<service>` actions to run when the
service's generation changes (it existed in the old profile and still
exists in the new one). Same action values as `init`.

Default: `["reload"]`.

#### `removed`

An ordered list of actions to run when the service disappears from the
new profile (it existed in the old profile but not in the new one).

The `removed` actions are read from the **old generation's** config
(the generation that shipped the service). This ensures the package
author controls how their own service is torn down, even if the new
generation knows nothing about it.

These actions are executed using the old generation's init script path:

```sh
$PROFILE_OLD_GENERATION/etc/init.d/<service> <action>
```

This is necessary because `/etc/init.d/<service>` may already be gone by
the time the orchestrator runs.

Default: `["stop"]`.

#### `reboot_required`

Whether a system reboot is required for this service to pick up
changes. In this iteration, the field is metadata for future handling
and does not suppress any configured actions.

Default: `false`.

#### `upgrade_if_status`

Controls whether the orchestrator should execute the `upgrade` actions
after it detects that a service changed generation.

Before running `upgrade`, the orchestrator calls:

```sh
/etc/init.d/<service> status
```

Supported values:

- `running`: run `upgrade` only when the service status indicates that
  the service is running
- `stopped`: run `upgrade` only when the service status indicates that
  the service is not running
- `always`: always run `upgrade`, regardless of service status

Default: `running`.

#### Examples

A boot-only service like `nix-mounter` that should never be touched
by the orchestrator on upgrade:

```json
{
  "init": ["boot"],
  "removed": [],
  "upgrade": []
}
```

The `bmc` compositor service that should be reloaded on upgrade
(just uses defaults, config file can be omitted entirely):

```json
{
  "init": ["boot", "start"],
  "removed": ["stop"],
  "upgrade": ["reload"],
  "upgrade_if_status": "running"
}
```

A service requiring a full system reboot:

```json
{
  "init": [],
  "removed": ["stop"],
  "upgrade": [],
  "reboot_required": true
}
```

### nix-activator

This service activates the bmc profile on every boot. This ensures
that the system is in a defined state. It also supports activating a
new generation when BOS has been upgraded. When that happens, the
profile is not activated before boot like normally, but only
afterwards by this service.

#### Boot-time generation semantics

The activation entrypoint auto-derives both generation paths. Callers
do not need to set `PROFILE_NEW_GENERATION` or `PROFILE_OLD_GENERATION`:

- `PROFILE_NEW_GENERATION` is derived from the entrypoint's own
  filesystem path (`dirname(dirname(entrypoint_dir))`).
- `PROFILE_OLD_GENERATION` is derived from the `current` symlink in
  the profile directory.

On boot, `current` already points to the generation being activated.
The entrypoint derives old == new. The orchestrator sees all services
as unchanged and executes no actions. This is correct — OpenWRT rc.d
has already called `boot()` on all enabled services via their `S*`
links.

On upgrade (non-boot activation), `current` still points to the
previous generation when the entrypoint runs. The entrypoint derives
old != new. The orchestrator diffs the generations and executes
init/upgrade/removal actions as configured.

The critical invariant: the `current` symlink is updated only *after*
activation completes. This is what makes old-generation derivation
work for upgrades while being harmlessly identity for boot.

Callers may override either env var if they have a specific reason.
For example, the upgrade code path sets `PROFILE_NEW_GENERATION`
explicitly to point to a generation that is not yet `current`.

### bmc-nix-service-orchestrator

This is the script that restarts other services. It is launched
during activation (as one of the ordered activation scripts) and
then waits for the profile lock to become available before running the
init/upgrade actions for each service that changed.

The orchestrator is a one-shot script. It doesn't matter what the
previous version looked like, so it lives outside the 'replacement'
concept. The activation always starts the current version from the
new generation's store path.

The orchestrator is not an OpenWrt service. Instead, the activation
registers it as a one-shot procd instance via `ubus call service set`.
Procd then spawns and manages the process, fully decoupled from the
activation — which is important because the activation itself might
be running under an OpenWrt service that is being upgraded.

Before registering the new one-shot instance, the activation should
ensure a stale `bmc-nix-service-orchestrator` instance is not still
present. It should explicitly query procd and delete or terminate the
previous instance when it exists, rather than relying on `service set`
to replace it safely.

The activation passes the old generation, new generation, current-link
path, instance name, and timeout as arguments. The orchestrator then
diffs the two generations, waits for the profile lock, reacquires it,
and verifies that `current` points to the new generation. Only then
does it determine which services need init/upgrade actions and apply
the `upgrade_if_status` gate for changed services. If `current` does
not point to the new generation after the lock is acquired, the
orchestrator exits with an error and runs no service actions.

The activation entrypoint itself participates in profile locking.
Callers that already hold the profile lock export
`ACTIVATION_HAS_PROFILE_LOCK=1`, and the shell entrypoint trusts that
flag and skips self-locking. Callers that do not already hold the
profile lock make the shell entrypoint open `<profile_dir>/.lock` and
attempt a non-blocking `flock -n`. If the profile is already locked,
activation aborts immediately instead of waiting, because a concurrent
profile mutation makes the request stale.

```sh
service_name="bmc-nix-service-orchestrator"
binary="/nix/store/<hash>-bmc-nix-service-orchestrator/bin/bmc-nix-service-orchestrator"

ubus call service set "{
  \"name\": \"$service_name\",
  \"instances\": {
    \"main\": {
      \"command\": [
        \"$binary\",
        \"--old-generation\",
        \"$PROFILE_OLD_GENERATION\",
        \"--new-generation\",
        \"$PROFILE_NEW_GENERATION\",
        \"--current-link\",
        \"/nix/var/nix/gcroots/profiles/bmc/current\",
        \"--instance-name\",
        \"$service_name\",
        \"--timeout-seconds\",
        \"300\"
      ],
      \"stdout\": true,
      \"stderr\": true
    }
  }
}"
```

Without a `"respawn"` key the process runs once and is cleaned up
when it exits. Stdout and stderr are forwarded to logd.

### bmc

BMC itself, either the monolithic application or later the compositor
is a service that will exist since the beginning of the switch to Nix
architecture as well. It will work the same as the current `bmc`
service, except that it will start `bmc` from the /nix/store.
