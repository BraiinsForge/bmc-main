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
04. **Start upgrade server** — launches `nix run .#upgrade-server` in the background with one
    `--package NAME=VERSION=STORE_PATH` per built package, waits until both the cache (`/nix-cache-info`) and the index
    (`nix-package-index.v1.json`) answer HTTP, and reads the cache public key from the keypair directory
    (`$XDG_STATE_HOME/bmc-upgrade-server`). Server output goes to `bmc-upgrade-server.log` in the system temp directory.
    The advertised host address is autodetected from the route to the device.
05. **Register server on device** — runs `bmc-nix-cli register-server` on the device with id `dev-upgrade`, pointing
    both the index document URL (`--index-url`) and the cache substituter at the developer machine. The index is
    unsigned, so the index public key mirrors the cache key, matching the command `upgrade-server` itself prints. The
    registration is `--exclusive`: it disables every other server entry, leaving the factory entry alone.
06. **Only the harness server resolves** — asserts the exclusivity took, rather than assuming it. A public entry left
    enabled decides the upgrade whenever it publishes a higher version, because resolution ranks a candidate's version
    above its server's priority; and a `required` entry the device cannot reach fails the whole `CheckForUpgrade` probe.
    Registration seeds the registry from the shipped default when the runtime file is missing, so production entries
    turn up unprompted (#BDK-666).
07. **Authenticate** — `AuthenticationService/Login` via grpcurl (plaintext h2c on port 80); the returned token becomes
    the `session_id` cookie for the authenticated `UpgradeService` calls.
08. **Check for upgrade** — expects a populated `packages` plan, an `APP_RESTART` disruption, and an upgrade id.
09. **Run upgrade** — `StartUpgrade` with the upgrade id, consuming the progress stream live; the stream must pass
    through the `REALIZING` and `ACTIVATING` package phases and end with a `finished` event.
10. **Profile advanced** — re-reads the current generation, requiring it to have incremented, and requires every served
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

The run is self-cleaning device-side — the reinstall returns the device to its baseline — but the `dev-upgrade` server
registration persists exactly as with `upgrade-e2e`, so the cleanup below applies.

## Cleanup

**Remove the registration once you stop the harness, or the device's upgrade checks break.** The `dev-upgrade` server
registration persists on the device after the run: its entry stays in `/etc/nix-upgrade/servers.json` and its
substituter plus trusted key stay in `/etc/nix/nix.conf`. Index fetching aborts when any *required* top-level server
fetch fails (`fetch_and_merge_indexes` in `bmc-nix/src/index.rs`; only servers registered as optional degrade to a
warning), and `register-server` marks servers required by default, so while `dev-upgrade` is still registered and
enabled but the developer machine's server is down, *every* `CheckForUpgrade` on the device fails entirely — not just
for `dev-upgrade` — until the entry is removed or disabled. Re-running the harness against a live server heals it
(`register-server` replaces the entry by id).

**The run also leaves every other server entry disabled.** Registration is `--exclusive`, which sets `"enabled": false`
on each entry it did not register — the production `forge` entry included — and nothing re-enables them when the run
ends. Re-enable them by hand once you are done, or the device goes on resolving upgrades against the developer machine
alone, which by then is gone. The factory entry is never touched.

To remove it, on the device:

- delete the `dev-upgrade` object from the `servers` array in `/etc/nix-upgrade/servers.json`;
- delete the `extra-substituters = <cache-url>` and `extra-trusted-public-keys = <key>` lines from `/etc/nix/nix.conf`.

These undo exactly what the "Register server on device" stage wrote (`register-server` adds the `servers.json` entry and
sets those two `extra-*` lines). Disabling the entry (`"enabled": false` in `servers.json`) instead of deleting it also
stops the failing fetches while keeping the registration around for a later run.
