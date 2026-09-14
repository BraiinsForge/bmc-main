# Boser upgrade E2E rig over REST

This command serves the host side of a BMM101 upgrade test and then drives one upgrade through Boser's REST API:
`POST /api/v1/upgrade/check`, `POST /api/v1/upgrade/start` and the `GET /api/v1/upgrade/state/events` stream. It
installs nothing and stops no service on the device; its only SSH use reads device state to verify the upgrade
afterwards. The operator prepares the device; the runner talks to Boser only after the operator confirms that
preparation.

## Inputs

- The device address of Boser's web API (`--device`), and its root password (`--password`, empty by default).
- The device's root SSH address (`--ssh`), used only to read its boot id, `/etc/bos_version` and the Nix profile.
- A complete, flattened `nix-package-index.v1.json` for the packages intended for this device. Realize every package
  store path on the host first. Include every currently installed system package; an incomplete index can fail a check.
- The device's exact full BOS version, not just the release number.
- This host's device-reachable address.
- For firmware testing, a compatible BMM101 sysupgrade tarball with a later release number. A different build hash of
  the same release is not a firmware upgrade to Boser's resolver.
- Package names to install on top of the upgrade (`--packages`), matching the catalog the runner prints.
- A Boser binary honouring the `BOS_INDEX_URL` override.

The rig rejects child `indexes` rather than following production feeds. It passes the package index to the existing
upgrade server without changing package `bmc_version` or other metadata. The existing server may rewrite store-backed
asset URLs for HTTP serving. Top-level cache hints are not used by the current package resolver; device Nix
configuration still controls substituters. This is controlled offer hosting, not an enforced network-egress sandbox.

## Start the host

Run from the BMC worktree:

```sh
nix run .#deck -- boser-upgrade-e2e \
  --device 192.168.1.20 \
  --ssh 192.168.1.20 \
  --package-index /path/to/nix-package-index.v1.json \
  --running-version 2026-08-01-0-acde0123-26.08-plus \
  --serve-ip 192.168.1.10 \
  --packages weather
```

Replace the example versions and addresses with the actual values. This serves a firmware index containing only the
running release, so firmware cannot take precedence over package upgrades. Add `--image /path/to/firmware.tar` to offer
firmware instead. The image must be a BMM101 eMMC sysupgrade.

The firmware fixture and package-only tests are intentionally limited to BMM101 booting from eMMC. NAND/full-miner and
Deck firmware upgrades are outside this rig.

Defaults: signed Nix cache on TCP 8080, package index on 8081, BOS firmware index/image on 8082. Override these with
`--port`, `--index-port`, `--firmware-port`. Allow the chosen ports through the host firewall for the device. The runner
does not change firewall rules. Keep any temporary firewall rule scoped to this test and remove it afterwards. Ensure
these ports are free before starting: the existing readiness probes do not prove listener ownership if another process
is already serving on a selected port.

The command uses the existing upgrade server's signing key location. It retains each run's firmware fixture, package log
and generated operator instructions in a unique directory under `.tmp/boser-e2e/`. No automatic deletion or real-device
completion claim is made when the host process ends.

## Operator-controlled device preparation

Before running any printed command:

1. Arrange recovery access and preserve the normal binary/service configuration.
2. Confirm the device is a BMM101 booting from eMMC and its full version matches the fixture.
3. Snapshot `/etc/nix-upgrade/servers.json` and `/etc/nix/nix.conf`, preserving whether each existed.
4. Pause Boser automatic upgrades and BMC automatic upgrades/GC. Stop normal Boser before launching the test binary.
   Keep only one Boser process. Keep sysupgrade's normal image/signature checks enabled.
5. Use the printed `bmc-nix-cli register-server --exclusive` command. Do not keep old test registrations as your
   baseline.
6. Launch Boser with the environment variable from the printed instructions:

```sh
BOS_INDEX_URL=http://192.168.1.10:8082 /path/to/boser-openwrt --log-to-file
```

Boser requests `index.v2.json`; BMC's firmware fixture remains v1. Package indexes remain v1.

## The driven upgrade

Type `ready` in the runner once Boser serves. The runner then logs in, prints the installable catalog, checks with the
requested packages and prints the capability, the previews and the offer (its ID, kind and disruption). It starts the
offer only after you type `yes`, then follows the state stream and prints each snapshot as `STATE`, `STATE/PHASE` or
`STATE/PHASE/downloaded_bytes` until `COMPLETED`, `REBOOTING` or `FAILED`. A `FAILED` snapshot ends the run with the
device's phase and reason; a stream that goes quiet or exceeds `--stream-deadline` (900 s) ends it too. Package-only
tests require an initialized Nix installation. Initial Nix bootstrap through sysupgrade's factory feed/tarball is not
supplied by this runner.

After the terminal state the runner verifies the device over SSH and exits non-zero on any disagreement with the offer.
A check that reports an `UNHEALTHY` package store stops the run before anything starts. `COMPLETED` must leave the boot
id unchanged and is not accepted for a firmware offer. After `REBOOTING` it waits up to `--reboot-deadline` (600 s) for
a new boot id and for a Boser that logs in and reports no execution. It then compares `/etc/bos_version` with the
offered firmware, requires a newer profile generation whose manifest carries every offered package version (an unchanged
generation when no package changed), and repeats the check: the same request must no longer produce an offer. After a
reboot that check reaches whichever Boser the new firmware starts. The registered package server persists in
`servers.json`, a `BOS_INDEX_URL` override does not, so its firmware side consults the device's default feed.

For no-upgrade coverage, use a package index matching the installed profile and omit the firmware image; the runner
reports the absent offer and starts nothing. For install coverage, include the new package metadata and store path in
the index and pass `--packages <name>`. For cached-offer coverage, answer `no` at the start prompt, make a newer fixture
available, and start the original offer ID with a plain `curl` against `POST /api/v1/upgrade/start`; a stale ID answers
409 while a run holds admission and 404 otherwise. The host runner snapshots its package index at startup, so changing
the original input file alone does not change the served index.

Keep the host servers and Boser alive through the operation. Ending the runner does not cancel accepted work, but
killing Boser or losing its foreground SSH session can terminate the daemon. An uncertain failure is not proof of
quiescence: do not restart services or restore configuration while sysupgrade or package activation may still run.

After confirmed completion or confirmed stopped failure, restore the two configuration snapshots and the normal
binary/services/maintenance settings. Verify the device is back on its intended production configuration. Only then type
`stop` in the host runner. Ctrl-C/EOF also stops hosting, but neither certifies that the device operation has stopped.

This exercises the REST check/start/state contract BMC consumes, backend catalog/check/start/watch and platform
execution. BMC display propagation, automatic-upgrade/GC handover and hardware results require separate verification.
