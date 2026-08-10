# Firmware and Application Upgrade Interlinking

> **A firmware upgrade always includes an application package upgrade.** They are not two independent operations that
> happen to be offered together. The incoming firmware tarball makes the package upgrade part of `sysupgrade` itself.

`bmc-main` owns the upgrade API and starts a firmware upgrade by invoking `/sbin/sysupgrade`. During image validation,
OpenWrt loads the `COMMAND` file from that tarball and calls its `package_check_image` function. That function runs the
tarball's own `bmc-nix-cli upgrade` before OpenWrt flashes the firmware.

The package command is still invoked when the device already has the latest packages. In that case the package planner
finds no changes and the CLI completes without building another profile generation. This no-op is the only normal case
where a firmware upgrade does not stage different application packages.

For package resolution and deferred activation details, see [`upgrades.md`](upgrades.md). For the complete tarball
contract, including first-time Nix initialization, see [`openwrt-tarball.md`](openwrt-tarball.md).

## Responsibilities Across Repositories

| Repository                | Responsibility                                                                                                                                                                                                                                          |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `bmc-main`                | Implements the upgrade API, checks for firmware and package updates, downloads the firmware, and starts `/sbin/sysupgrade`. It also builds the statically linked `bmc-nix-cli` used by the firmware build.                                              |
| `bos-main`                | Assembles the firmware-build Nix payload. It takes `bmc-nix-cli` from its `bmc-main` flake input and renders `servers.json.default` from the `bos-packages` template, then supplies both to the OpenWrt build. It is not part of the runtime Rust path. |
| `bos-packages`            | Owns the OpenWrt `bmc` package and the `servers.json.in` template used to produce the default package-server registry.                                                                                                                                  |
| `openwrt`                 | Implements `sysupgrade`, packs the external Nix payload into the image, extracts and sources the tarball's `COMMAND`, calls `package_check_image`, and flashes only after that check succeeds.                                                          |
| Incoming firmware tarball | Supplies `COMMAND`, the latest statically linked `bmc-nix-cli` built for that firmware, and `servers.json.default`. Its command performs the package upgrade.                                                                                           |

The important ownership boundary is between runtime and firmware construction. No Rust code from `bos-main` participates
in this application's upgrade flow. `bmc-main` starts `sysupgrade`; the payload assembled earlier by the `bos-main`
firmware build makes the package step mandatory inside that command.

## Execution Order

For a normal firmware upgrade on a Nix-initialized device:

1. `bmc-main` invokes `/sbin/sysupgrade <image>`.

2. OpenWrt's firmware validation reaches `platform_check_image`.

3. `platform_check_image` extracts and sources the incoming tarball's `COMMAND`, then calls `package_check_image`. The
   function is named `package_check_image`, not `package_image_check`.

4. `package_check_image` extracts `bmc-nix-cli` and `servers.json.default` from the same tarball.

5. The extracted CLI runs before the flash:

   ```sh
   bmc-nix-cli upgrade \
       --default-servers-config <staged>/servers.json.default \
       --firmware <incoming-bos-version> \
       --next-boot
   ```

   When `bmc` has handed explicit application installs to the firmware path, the command also receives
   `--install-from <handoff>`.

6. The CLI resolves packages for the incoming firmware, realizes any missing store paths, and stages a profile
   generation for the next boot. If the resolved profile is unchanged, this step succeeds as a no-op.

7. Only after the package command succeeds does OpenWrt proceed with flashing the firmware.

8. After reboot, the incoming firmware activates the staged profile. Package resolution and download do not wait until
   after the flash; only activation is deferred.

If package resolution, realization, or profile staging fails, `package_check_image` fails and `sysupgrade` aborts before
flashing. A firmware upgrade therefore cannot bypass a broken package-upgrade path.

## Why the Tarball Carries the CLI

Every released firmware tarball contains the latest `bmc-nix-cli` selected by that firmware build, statically compiled
for the target. In `bmc-main`, `workspace.nix` builds the `bmc-nix-cli-armv7-release` output with the ARMv7 musl
profile. The `bos-main` firmware build selects that output, renders `servers.json.default` from
`bos-packages/bmc/files/servers.json.in`, and exposes both files to OpenWrt as the external Nix payload. The OpenWrt
image builder then copies that payload into the sysupgrade tarball.

`COMMAND` executes the extracted binary from its staging directory. It does not rely on the version installed in the
outgoing firmware. This keeps the command-line contract and package logic coupled to the incoming image that expects
them.

"Latest" here means the CLI version bundled by the incoming firmware build. It does not mean that `sysupgrade` downloads
a newer CLI separately at installation time.

## Consequences for Upgrade Checks

Firmware and package availability may be probed independently, but a firmware offer subsumes a package-only offer
because applying the firmware necessarily runs the package upgrade. Running the ordinary package upgrade first would
only resolve and apply work that the firmware path resolves again for the incoming BOS version.

The package probe is also a pre-flight for mandatory work inside `sysupgrade`. A package-probe failure must not be
hidden merely because firmware is available: the tarball will still enter the same package layer before it can flash.

## Source Locations

- `bmc-main`: `bmc/src/web/grpc/upgrade_service.rs`, `bmc-openwrt/src/manager.rs`, `bmc-nix/src/bin/cli.rs`,
  `bmc-nix/src/upgrade.rs`, and `workspace.nix`.
- `bos-main`: `braiins-os-plus/openwrt.nix` and `braiins-os-plus/defaults/stm32mp15_ii3/release.conf`.
- `bos-packages`: `bmc/files/servers.json.in` and `bmc/Makefile`.
- `openwrt`: `package/base-files/files/sbin/sysupgrade`, `target/linux/stm32mp15/base-files/lib/upgrade/platform.sh`,
  `target/linux/stm32mp15/image/COMMAND`, and `target/linux/stm32mp15/image/Makefile`.

When changing any of these paths, preserve the invariant: starting a released firmware upgrade must also invoke the
tarball's package upgrade, even when that invocation ultimately reports that the installed profile is already current.
