# Interactive Boser upgrade E2E rig

This command prepares the host servers for BMM101 testing through the temporary Boser console. It does not SSH to the
device, install anything, stop services or start an upgrade. The operator drives the device.

## Inputs

- A complete, flattened `nix-package-index.v1.json` for the packages intended for this device. Realize every package
  store path on the host first. Include every currently installed system package; an incomplete index can fail a check.
- The device's exact full BOS version, not just the release number.
- The device boot mode (`emmc` or `sd`) and this host's device-reachable address.
- For firmware testing, a compatible BMM101 sysupgrade tarball with a later release number. A different build hash of
  the same release is not a firmware upgrade to Boser's resolver.
- A Boser binary containing the `upgrade-console` feature and the `BOS_INDEX_URL` override.

The rig rejects child `indexes` rather than following production feeds. It passes the package index to the existing
upgrade server without changing package `bmc_version` or other metadata. The existing server may rewrite store-backed
asset URLs for HTTP serving. Top-level cache hints are not used by the current package resolver; device Nix
configuration still controls substituters. This is controlled offer hosting, not an enforced network-egress sandbox.

## Start the host

Run from the BMC worktree:

```sh
nix run .#deck -- boser-upgrade-e2e \
  --package-index /path/to/nix-package-index.v1.json \
  --running-version 2026-08-01-0-acde0123-26.08-plus \
  --serve-ip 192.168.1.10 \
  --boot-mode emmc
```

Replace the example version and address with the actual values. This serves a firmware index containing only the running
release, so firmware cannot take precedence over package upgrades. Add `--image /path/to/firmware.tar` to offer firmware
instead. The image must match the BMM101 board and boot mode.

Defaults: signed Nix cache on TCP 8080, package index on 8081, BOS firmware index/image on 8082. Override these with
`--port`, `--index-port`, `--firmware-port`. Allow the chosen ports through the host firewall for the device. The runner
does not change firewall rules. Keep any temporary firewall rule scoped to this test and remove it afterwards. Ensure
these ports are free before starting: the existing readiness probes do not prove listener ownership if another process
is already serving on a selected port.

The command uses the existing upgrade server's signing key location. It retains each run's firmware fixture, package log
and generated operator instructions in a unique directory under `.tmp/boser-e2e/`. No automatic deletion or real-device
completion claim is made when the host process ends.

## Operator-controlled device session

Before running any printed command:

1. Arrange recovery access and preserve the normal binary/service configuration.
2. Confirm the device's full version and boot mode match the fixture.
3. Snapshot `/etc/nix-upgrade/servers.json` and `/etc/nix/nix.conf`, preserving whether each existed.
4. Pause Boser automatic upgrades and BMC automatic upgrades/GC. Stop normal Boser before launching the console binary.
   Keep only one Boser process. Keep sysupgrade's normal image/signature checks enabled.
5. Use the printed `bmc-nix-cli register-server --exclusive` command. Do not keep old test registrations as your
   baseline.
6. Launch the special binary with both environment variables from the printed instructions:

```sh
BOS_INDEX_URL=http://192.168.1.10:8082 BOSER_UPGRADE_CONSOLE=1 \
  /path/to/boser-openwrt --log-to-file
```

Boser requests `index.v2.json`; BMC's firmware fixture remains v1. Package indexes remain v1.

In the console, run `catalog`, then `check` or `check package-name ...`. Inspect capability, selected kind and previews.
Only run `start <offer-id>` and then `yes` when the offer is the intended one. Watch snapshots print automatically.
Package-only tests require an initialized Nix installation. Initial Nix bootstrap through sysupgrade's factory
feed/tarball is not supplied by this runner.

For no-upgrade coverage, use a package index matching the installed profile and omit the firmware image. For install
coverage, include the new package metadata and store path in the index and use `check <name>`. For cached-offer
coverage, first check an offer, then make a newer fixture available without issuing another check; confirm that starting
the original offer uses the previously checked package set. The host runner snapshots its package index at startup, so
changing the original input file alone does not change the served index.

Keep the host servers and Boser alive through the operation. Console EOF/quit does not cancel accepted work, but killing
Boser or losing its foreground SSH session can terminate the daemon. An uncertain failure is not proof of quiescence: do
not restart services or restore configuration while sysupgrade or package activation may still run.

After confirmed completion or confirmed stopped failure, restore the two configuration snapshots and the normal
binary/services/maintenance settings. Verify the device is back on its intended production configuration. Only then type
`stop` in the host runner. Ctrl-C/EOF also stops hosting, but neither certifies that the device operation has stopped.

This exercises backend catalog/check/start/watch and platform execution. External API/authentication, BMC display
propagation, automatic-upgrade/GC handover and hardware results require separate verification.
