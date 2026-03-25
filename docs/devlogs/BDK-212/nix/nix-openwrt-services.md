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

After the initializer activates the profile, it writes the `nix_init`
U-Boot env var for the `nix-factory-reset` service.

The initializer also handles the initial servers.json config. It has a
config embedded in itself and it copies the default config from
firmware if it exists.

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
happens, it removes the /mnt/data/nix under /mnt/data. There will be a
mechanism to prevent this for development, `/mnt/data/NIX_INHIBIT_INIT`.

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

However, it might not be feasible to restart all of the services, so
it is possible to inhibit this behavior by providing a restart-inhibit
file at a given location under the package.

Crucially, it is realized that the activation scripts themselves may
be ran by such OpenWrt service managed by Nix. Implying that a restart
done during the activation would potentially lead to killing the
activation script itself. This is why the restart of the services will
also be done by a service. This service will receive as arguments:
- The PID of the activation process (the main activation script, not the individual ones)
- The services to stop (no longer existing services)
- The services to restart
- The services to start (new services)
This is implemented through the `nix-service-orchestrator` service.

Then, during activation, this service is started. It is the only
service that is (re)started during activation itself. After the
service sees that the activation process is finished, it performs the
restarts.

### Package services structure

Each package can provide OpenWrt services in `etc/init.d`. These
services will be automatically symlinked to `/etc/init.d` on the
system, so they will be picked up by OpenWrt on boot. There might also
be started before the boot through the `nix-service-orchestrator`.

To inhibit a restart of a service when restart is not supported,
the package shall provide `etc/init.d.conf/<service>-restart-inhibit`
file. This inhibits restart of `<service>`

### Services collection

There will be a hook that will collect all the services in
etc/profile.d of the services. It will collect the information about
restart inhibits as well. It produces a single json file with all the
information.

Then, a single activation binary will collect this file and read it to
assess if the services have been changed. It knows so by comparison of
the new and old service files themselves. The services of the same name
are used. This activation script then starts the `nix-service-orchestrator`
and sends it the arguments through a file under `/tmp`.

### nix-activator

This service activates the bmc profile on every boot. This ensures
that the system is in a defined state. It also supports activating a
new generation when BOS has been upgraded. When that happens, the
profile is not activated before boot like normally, but only
afterwards by this service.

### nix-service-orchestrator

This is the service that restarts other services. It doesn't do
anything when it starts on boot. Only when it is started by the
activation, it will stop, restart and start given services after
the activation has finished.

The orchestrator service is a one shot service. It doesn't matter what
the previous version of the service was, so this service lives outside
of the 'replacement' concept. This service is always just started by
the activation scripts, even if previous orchestrator service looked
differently.

The reason for this orchestrator being an OpenWrt service is that we
cannot permit something killing the activation process itself.
The activation process itself might be running under an OpenWrt service
that is being upgraded.

NOTE: maybe this doesn't have to be a service, we can also use procd
directly to start it. The only requirement here is that it is not
coupled to the activation scripts.

### bmc

BMC itself, either the monolithic application or later the compositor
is a service that will exist since the beginning of the switch to Nix
architecture as well. It will work the same as the current `bmc`
service, except that it will start `bmc` from the /nix/store.
