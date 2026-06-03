# BOS Avahi Advertisement Package Design

## Context

BDK-506 needs the fleet-management widget to discover BOS+ miners on the
local network through mDNS. This design covers only the BOS-side
advertisement prerequisite: an ARMv7 Nix package that installs and runs
Avahi, and conditionally advertises the BOS miner HTTP service on miner
platforms.

The package must use `armv7Pkgs.avahi` from the current flake. It must not
use the `nixpkgs#` flake registry.

## Goals

- provide a separate ARMv7 package for Avahi support, not part of `core`
- start Avahi through an OpenWrt init.d service on boot
- use a minimal Avahi daemon config suitable for the target OpenWrt system
- create the `avahi` user and group during activation without `useradd`
- advertise BOS HTTP service only on miner platforms
- keep the activation behavior idempotent

## Non-Goals

- implement fleet-management discovery or telemetry polling
- support non-BOS miner families
- add a per-device configuration UI
- require dbus on the target device
- statically copy `bos.service` into `/etc/avahi/services`

## Package Shape

Add a separate package named `bos-avahi` under `nix/pkgs`. Expose it from
`nix/packages.nix` as its own package entry so it can be included in ARMv7
profile generation independently from `core`.

The package uses the existing `mkPackage` helper and contains:

- the nixpkgs Avahi runtime package from `armv7Pkgs.avahi`
- one OpenWrt init.d service for `avahi-daemon`
- one minimal Avahi daemon config copied to `/etc/avahi/avahi-daemon.conf`
- one activation script for user/group setup and conditional service file
  generation

The package does not ship `/etc/avahi/services/bos.service` through
`copyFiles`. That file is created only by activation after platform
detection.

## OpenWrt Service

The Avahi daemon service may run on any device. Platform gating applies only
to the BOS advertisement file, not to the daemon itself.

The service should use the existing OpenWrt service helper, preferably
`mkOpenWrtDaemon`, and run Avahi with an explicit config path:

```sh
avahi-daemon --daemonize=no --file=/etc/avahi/avahi-daemon.conf
```

The concrete executable path should point into `armv7Pkgs.avahi`, for
example `${armv7Pkgs.avahi}/bin/avahi-daemon`.

The service should be enabled by default so OpenWrt starts Avahi on boot.
The existing Nix service orchestrator may use its default lifecycle behavior
unless implementation finds Avahi needs a narrower service config.

## Avahi Config

Ship a minimal config at `/etc/avahi/avahi-daemon.conf`:

```ini
[server]
use-ipv4=yes
use-ipv6=yes
check-response-ttl=no
use-iff-running=no
enable-dbus=no

[publish]
publish-addresses=yes
publish-hinfo=no
publish-workstation=no
publish-domain=yes

[rlimits]
rlimit-core=0
rlimit-data=4194304
rlimit-fsize=0
rlimit-nofile=30
rlimit-stack=4194304
rlimit-nproc=3
```

`enable-dbus=no` is required because the target OpenWrt system has no dbus.

## Activation

The activation script must be POSIX `/bin/sh` and BusyBox-compatible. It
must not rely on `useradd`, `groupadd`, GNU-only flags, Python, Perl, or
other tools that are not guaranteed on the device.

The script ensures:

- `/etc/group` contains an `avahi` group with GID `100`
- `/etc/passwd` contains an `avahi` user with UID `100` and GID `100`
- `/etc/avahi/services` exists
- `/etc/avahi/services/bos.service` is present only on miner platforms

The user and group entries should be created only when missing. If an
`avahi` entry already exists, activation should leave it in place rather
than rewriting user-managed state.

Use these intended entries when creating missing records:

```text
avahi:x:100:
avahi:x:100:100:avahi:/var/run/avahi-daemon:/bin/false
```

The activation script reads `/etc/bos_platform`. A device is a BOS miner
when the platform string contains `bmm1` or `bfm1`. On miner platforms, the
script writes `/etc/avahi/services/bos.service`. On all other platforms, or
when `/etc/bos_platform` is absent or unreadable, the script removes
`/etc/avahi/services/bos.service` if it exists.

## Advertised Service

The generated `bos.service` file advertises the BOS miner HTTP API:

- service name: `%h`, with wildcard replacement enabled
- service type: `_http._tcp`
- subtype: `_bos._sub._http._tcp`
- port: `80`

The service XML should match Avahi's service file schema and include a
single service group.

Expected shape:

```xml
<?xml version="1.0" standalone='no'?>
<!DOCTYPE service-group SYSTEM "avahi-service.dtd">
<service-group>
  <name replace-wildcards="yes">%h</name>
  <service>
    <type>_http._tcp</type>
    <subtype>_bos._sub._http._tcp</subtype>
    <port>80</port>
  </service>
</service-group>
```

## Data Flow

1. The package is installed into the active Nix profile.
2. Activation runs the package activation script.
3. Activation creates the Avahi user/group if needed.
4. Activation ensures the Avahi service directory exists.
5. Activation reads `/etc/bos_platform`.
6. Miner platforms get `/etc/avahi/services/bos.service`; non-miner
   platforms do not.
7. The service orchestrator enables and starts the OpenWrt Avahi service.
8. Avahi reads `/etc/avahi/avahi-daemon.conf` and service files under
   `/etc/avahi/services`.

## Error Handling

Activation should fail loudly for file write failures that prevent the
package from establishing its required runtime state. Missing
`/etc/bos_platform` is not an error; it means the device should not advertise
the BOS miner service.

If `/etc/passwd` or `/etc/group` exists but cannot be written, activation
should exit nonzero. If an existing `avahi` user or group has a different ID,
activation should leave it unchanged and continue. This avoids overwriting
device-local state while still supporting systems that already provide
Avahi identities.

## Verification

Implementation should verify:

- Nix evaluation/build of the new package succeeds for ARMv7
- the package output contains the OpenWrt init.d service
- the package output contains activation scripts
- the minimal config is copied to the activation copy area for
  `/etc/avahi/avahi-daemon.conf`
- activation script shell syntax is valid
- activation creates `bos.service` when a fixture `/etc/bos_platform`
  contains `bmm1`
- activation creates `bos.service` when a fixture `/etc/bos_platform`
  contains `bfm1`
- activation removes or omits `bos.service` for non-miner platforms
- the generated service XML contains `_http._tcp`,
  `_bos._sub._http._tcp`, and port `80`

## Success Criteria

- `bos-avahi` is a separate ARMv7 package that uses nixpkgs Avahi from the
  current flake.
- Avahi starts on boot through an OpenWrt service.
- The Avahi config disables dbus.
- The `avahi` user and group are created during activation when missing.
- BOS HTTP mDNS advertisement appears only when `/etc/bos_platform`
  identifies a `bmm1` or `bfm1` miner.
- Non-miner devices may run Avahi but do not advertise `bos.service`.
