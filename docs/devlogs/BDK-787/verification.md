# Device verification — 8 September 2026

Tested BMM101 eMMC at 10.0.0.148 using BMC f8266586d and Boser 06aa3f6c1b. These identify the tested revisions, not the
heads after history cleanup and rebase.

- Installed a fresh test package, upgraded it, observed completion and verified active files.
- Checked against index v1, served v2, then started the original offer: it still installed v1.
- Ran a combined firmware/package upgrade. Firmware took precedence; sysupgrade reported Nix realizing, verifying and
  building before reboot. The new firmware and package generation were active afterwards, with the next-boot marker
  consumed. Its store path was already cached; the package-only run covered fresh downloads.
- Used firmware from [CI job 10225419](https://gitlab.ii.zone/bos/bos-main/-/jobs/10225419), MR !2559, revision
  f4f1512608c0f29b76f4d1c0b77911ab8fc34a6c. No firmware image was built locally.
- Restored the original packages and configuration, retaining the new firmware. Verified normal Boser, BMC and bosminer
  processes and stopped host test feeds.

Local evidence remains in .tmp/boser-e2e/device-148/ and .tmp/boser-e2e/device-148-combined/. These ignored artifacts
are not included in the MR.

The download-progress correction was added afterwards with regression tests: count archive transfers under
substitution/CopyPath activities, excluding metadata queries. The later rebase also preserved target-firmware feeds and
added E2E coverage for matching normalized firmware-index and package-feed versions. Those changes were not rerun on
hardware.

First-Nix bootstrap, firmware-failure recovery and external API/display wiring were not device-tested. GC,
automatic-upgrade handover and display-removal integration remain outside this verification.

## Automated verification

The locked dependency audit confirmed that core Boser, Unix, Buildroot and mockup pull in neither bmc-nix nor
bmc-upgrade; only OpenWrt uses them. Before documentation cleanup, both repository validation gates and the Boser ARM
release-tiny binary build passed. The final Boser native run passed 1,365 tests with 60 existing ignores; no tests were
disabled for this work. BMC's validation and focused upgrade, widget-mapping and mock package tests also passed.

At the original package-integration checkpoint, the ARM executable grew from 37,999,872 to 38,549,056 bytes: +549,184
bytes (+1.45%) against firmware baseline fc1b909f91. This is a historical executable-size measurement, not a measurement
of the later rebased head, firmware image or installed closure.
