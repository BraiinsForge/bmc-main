# End-to-End Package Upgrade Harness

`deck upgrade-e2e` drives a complete package upgrade cycle against a real Deck: it builds packages on the developer
machine, serves them to the device as a signed binary cache plus package index, and then exercises the production
upgrade path — `CheckForUpgrade` and `StartUpgrade` over the device's gRPC API — asserting that the bmc profile advanced
to a new generation. It is a development harness; nothing here ships on the device.

## Prerequisites

- A Deck with a Nix-initialized store (`nix run .#deck -- init`, then `nix run .#deck -- deploy`), running a bmc app
  that implements the `UpgradeService` RPCs (`CheckForUpgrade`, `StartUpgrade`).

- `/etc/nix-upgrade/servers.json` on the device — created automatically when missing. Factory provisioning normally
  writes it; a `deck init` device lacks it, and `register-server` (the "Register server on device" stage) then
  bootstraps the registry with the dev server doubling as the mandatory `factory` entry. Note the factory entry sticks
  after cleanup, so a bootstrapped device's factory points at the developer machine until reprovisioned.

- `grpcurl` on the developer machine's PATH (`nix shell .#pkgs.grpcurl`).

- The device must be able to reach the developer machine over the network: the harness serves the binary cache on
  `--port` (default 8080) and the package index on `--index-port` (default 8081), so those ports must not be firewalled.

## Invocation

```sh
nix run .#deck -- upgrade-e2e --device DEVICE_IP
```

With no `--packages`, the harness builds and serves every deck package — a superset of `deck deploy`'s default set. Use
the default set — a narrower `--packages` subset does not work today: the harness serves an index built solely from the
`--package` entries it publishes, and any installed system package that is absent from every consulted index is a hard
resolution failure, so `CheckForUpgrade` aborts. Serving a subset needs a baseline index carrying the rest of the
installed profile, which the harness does not yet plumb (`upgrade-server` accepts `--base-index`, but the harness never
passes it).

`--password` passes the device's web password to the gRPC `Login` call; the default empty string matches a device with
no password set. `--profile debug` serves the profiling build set, mirroring `deck deploy`.

An upgrade is only offered when the served build differs from the installed profile — a version bump or a store-path
change at the same version both count. Rebuilding from the same tree that deployed the device produces identical store
paths, so make a code change (or serve a different profile) before running the harness, otherwise the
`Check for upgrade` stage aborts with "no package upgrade offered".

**Deploy the current tree before the run.** Run `nix run .#deck -- deploy` first so the installed baseline is the latest
software, then make your code change and run `upgrade-e2e`. That way you are verifying that the newest software can
still upgrade itself; running the harness against a stale deployed baseline instead exercises some old-version upgrade
path that no longer reflects what ships.

## What the Stages Do

01. **grpcurl present / Device reachable / bmc-nix-cli present** — fail-fast preconditions; `bmc-nix-cli` is
    bootstrapped onto the device when missing, as in `deck deploy`.
02. **Resolve packages / Build packages** — the shared deploy stages; the closures are realized into the local
    `/nix/store` but, unlike `deck deploy`, never copied to the device.
03. **Snapshot current generation** — records the device's current profile generation number (from the `current` symlink
    in `/nix/var/nix/gcroots/profiles/bmc`) for the final assertion.
04. **Pre-run package config is the device's own / Capture server registry / Capture nix.conf** — refuses to start when
    `/etc/nix-upgrade/servers.json` still carries a `dev-upgrade` entry or `/etc/nix/nix.conf` still trusts a
    `dev-upgrade:*` key, because either means an earlier run did not restore the device's package config. Remove the
    registry entry, re-enable the servers it disabled, remove the rig URL token from `extra-substituters`, and remove
    the `dev-upgrade:*` token from `extra-trusted-public-keys`; then re-run. On clean config, the bytes of both files
    are recorded for the restore.
05. **Start upgrade server** — launches `nix run .#upgrade-server` in the background with one
    `--package NAME=VERSION=STORE_PATH` per built package, waits until both the cache (`/nix-cache-info`) and the index
    (`nix-package-index.v1.json`) answer HTTP, and reads the cache public key from the keypair directory
    (`$XDG_STATE_HOME/bmc-upgrade-server`). Server output goes to `bmc-upgrade-server.log` in the system temp directory.
    The advertised host address is autodetected from the route to the device.
06. **Register server on device** — runs `bmc-nix-cli register-server` on the device with id `dev-upgrade`, pointing
    both the index document URL (`--index-url`) and the cache substituter at the developer machine. The index is
    unsigned, so the index public key mirrors the cache key, matching the command `upgrade-server` itself prints. The
    registration is `--exclusive`: it disables every other server entry, leaving the factory entry alone.
07. **Only the harness server resolves** — asserts the exclusivity took, rather than assuming it. A public entry left
    enabled decides the upgrade whenever it publishes a higher version, because resolution ranks a candidate's version
    above its server's priority; and a `required` entry the device cannot reach fails the whole `CheckForUpgrade` probe.
    Registration seeds the registry from the shipped default when the runtime file is missing, so production entries
    turn up unprompted.
08. **Authenticate** — `AuthenticationService/Login` via grpcurl (plaintext h2c on port 80); the returned token becomes
    the `session_id` cookie for the authenticated `UpgradeService` calls.
09. **Check for upgrade** — expects a populated `packages` plan, an `APP_RESTART` disruption, and an upgrade id.
10. **Run upgrade** — `StartUpgrade` with the upgrade id, consuming the progress stream live; the stream must pass
    through the `REALIZING` and `ACTIVATING` package phases and end with a `finished` event.
11. **Profile advanced** — re-reads the current generation, requiring it to have incremented, and requires every served
    package that was installed before the upgrade to appear in the new generation's manifest at its served store path.
    Served packages absent from the pre-upgrade manifest are index-only — they are not auto-installed and not expected
    to appear.

The upgrade server is terminated when the procedure ends, whether it succeeded or aborted.

## Widget Install Variant

`deck install-widget-e2e` exercises the widget-install path over the same machinery: discovery via
`UpgradeService/GetInstallableWidgets`, planning via `CheckForUpgrade` with `installPackages`, and the shared
`StartUpgrade` stream. The plain upgrade e2e serves the full package set, so nothing is ever installable by
construction; this procedure creates the gap on purpose — it removes a widget from the device, then discovers and
installs it back:

```sh
nix run .#deck -- install-widget-e2e --device DEVICE_IP
```

`--widget` picks the package to remove and reinstall (default `widget-blockheight`); it must be a leaf widget the served
build contains, or the install is never offered. The prerequisites, `--packages`/`--profile`/`--password`/port flags,
and the full-package-set caveat above all apply unchanged. As with `upgrade-e2e`, deploy the current tree first
(`nix run .#deck -- deploy`) so the install runs against the latest software. Unlike `upgrade-e2e`, no code change
between deploy and run is needed: the removal itself creates the plan difference, so rebuilding the same tree works.

The shared preparation stages (preconditions through "Register server on device" and "Authenticate") are identical.
Then:

1. **Remove widget for reinstall** — `bmc-nix-cli remove-packages` on the device; skipped when the widget is already
   absent, so the procedure re-runs cleanly after a mid-flight abort.
2. **List installable widgets** — `GetInstallableWidgets` must offer the widget with its catalog metadata (uid, display
   name, category, icon).
3. **Check for install** — `CheckForUpgrade` with `installPackages: [WIDGET]` must plan the widget as an added change
   and return an upgrade id.
4. **Run upgrade** — the same `StartUpgrade` stream assertion as stage 8 above.
5. **Verify widget installed** — the widget is back in `list-packages` and its uid is exposed by
   `SceneManagementService/GetAvailableWidgets`, proving the running registry picked it up, not just the profile.

The run is self-cleaning device-side — the reinstall returns the device to its baseline — and the server registry and
`nix.conf` are captured and restored exactly as with `upgrade-e2e`, so the cleanup below applies.

## Cleanup

**The run cleans up after itself.** "Capture server registry" and "Capture nix.conf" record
`/etc/nix-upgrade/servers.json` and `/etc/nix/nix.conf` byte for byte before registration, and the run puts both back on
the way out whether it succeeded or failed — restoring the original bytes, or removing a file that was not there. So
neither the `dev-upgrade` entry, nor the production servers that `--exclusive` disabled, nor the rig's
`extra-substituters` and `extra-trusted-public-keys` lines outlive the run.

Restoring `nix.conf` matters most for the trusted key: `register-server` writes the rig's cache public key there, and a
key left behind is a standing grant for a developer machine's signing key on a device that will outlive the run.

That matters because the registry is unforgiving of a stale entry. Index fetching aborts when any *required* top-level
server fetch fails (`fetch_and_merge_indexes` in `bmc-nix/src/index.rs`; only servers registered as optional degrade to
a warning), and `register-server` marks servers required by default — so a `dev-upgrade` entry left behind pointing at a
developer machine that is now off would fail *every* `CheckForUpgrade` on the device, not just its own.

A restore failure fails the run rather than passing quietly, unless the run was already failing for another reason, in
which case it degrades to a logged warning so the original error survives. Either way both restores are attempted — one
blowing up does not skip the other, since half-restored is the state this exists to prevent.

A run killed outright can still leave its registration behind, and there the next run refuses to start: "Pre-run package
config is the device's own" aborts on a leftover `dev-upgrade` registry entry or trusted key rather than capturing it as
the baseline. Registration writes `nix.conf` first, so the trusted-key guard also covers a process killed before it
persisted `servers.json`.
