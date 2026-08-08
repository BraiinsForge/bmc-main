# Mock Upgrade Scenarios

`bmc-mock` can simulate every upgrade state the frontend has to handle, fully offline and selectable at runtime. The
mock runs the real `UpgradeService` → `SystemUpgradeService` pipeline; only the leaves are swapped: `MockIndex` serves
firmware offers, `MockPackageBackend` serves package upgrades, and a local throttled HTTP blob server stands in for the
firmware feed. Everything the frontend observes — check responses, progress streams, stream errors, the post-upgrade
"radio silence" — goes through the same code paths as on a device.

## Selecting a scenario

The scenario lives in `<mockfs>/etc/upgrade-scenario.json` (seeded from
`bmc-mock/mockfs-template/bmc100/etc/upgrade-scenario.json`). The file is re-read on every `CheckForUpgrade`, so it can
be edited while the mock runs; the next check picks it up. A missing file and missing fields fall back to the defaults
silently; unparseable content falls back with a logged warning.

```json
{
  "firmware": "available",
  "packages": "available",
  "run": "success"
}
```

| Field              | Values                                                              | Meaning                                                        |
| ------------------ | ------------------------------------------------------------------- | -------------------------------------------------------------- |
| `firmware`         | `available` (default), `up-to-date`, `check-error`                  | whether a firmware release is offered, or the check errors     |
| `packages`         | `available` (default), `unavailable`, `fetch-failed`                | whether a package upgrade is offered, or the index fetch fails |
| `run`              | `success` (default), `download-fail`, `hash-mismatch`, `apply-fail` | how a started upgrade run ends                                 |
| `store_free_bytes` | a byte count; omitted (default) leaves the store unconstrained      | free store space the mock reports to the daemon's preflight    |

The scenario selects states only — the offered firmware versions and package change lists are fixed built-in datasets,
not configurable. The previous releases in the firmware offer are display-only: their URLs point at the real feed and
are not served locally (the pipeline only ever runs the latest release).

Scenarios are check-scoped: `CheckForUpgrade` bakes the offer (URLs, hashes, upgrade id) from the scenario at check
time, and `StartUpgrade` consumes that stored offer. To exercise a different state, edit the file and check again.
`run: apply-fail` is the one execute-time knob — it is consulted when the apply actually runs.

`store_free_bytes` is the exception to "states only": the mock filesystem has no capacity to measure, so without an
explicit value the daemon's store preflight always passes and its `NotEnoughSpace` — the `FailedPrecondition` the
frontend sees — cannot be reached off-device. Set it below the plan's unpacked size plus 10% headroom to drive the
refusal; the value is re-read per call, so a full store can be freed mid-session like any other scenario field.

## How the run outcomes work

Firmware downloads come from a local blob server that serves a deterministic 24 MB image in 256 KiB chunks every 100 ms
(≈2.5 MB/s), so download progress is visible for a realistic stretch. The advertised sha256 matches the blob, and the
real pipeline downloads and verifies it. All simulated delays — the download throttle, the package progress steps, and
the sysupgrade wait — are the realistic default; `--fast-upgrades` drops them so the flows complete almost immediately.
Only a small reboot delay always remains, so the Applying event reaches the client before the simulated reboot kills the
process.

- `download-fail`: the offer points at the blob server's fail URL, which closes the connection before sending any
  response, so the failure surfaces as a download error. (A mid-stream drop would surface as a hash mismatch instead —
  the download loop treats a truncated body as a short file.)
- `hash-mismatch`: the offer advertises a wrong hash for the real blob; the full download completes, then verification
  fails.
- `apply-fail`: packages fail after the `Realizing` phase; firmware fails after download and verify, when the apply is
  handed off.

Failures terminate the `StartUpgrade` stream as gRPC status errors — there is no protobuf "failed" event, and the
frontend is expected to handle the stream erroring out.

## Firmware success closes the stream, then reboots

A successful firmware upgrade emits `FirmwareApplying` and then closes the `StartUpgrade` stream with a clean gRPC OK
status — the trailer flushes inside the shutdown-grace window that precedes the reboot. The device then reboots (on a
real device sysupgrade kills the process; if the reboot severs the connection before the clean close arrives, the client
treats a post-Applying disconnect as success too). The mock simulates the reboot: about 2 s after the apply hand-off it
logs `Mock sysupgrade: exiting to simulate the reboot` and exits with status 0. Use the supervisor recipe to get the
reboot-and-come-back experience:

```bash
just fe::serve-loop
```

It restarts the mock after each simulated reboot (plain `just fe::serve` stays down once the mock exits).

A package-only run (firmware `up-to-date`) completes in-process with a `Finished` event and no exit.

## E2E tests

`bmc-mock/tests/upgrade_scenarios.rs` covers the state matrix over native gRPC: each test spawns the compiled mock as a
subprocess, logs in (cookie auth), and asserts the wire behavior — check responses per scenario, package phase
sequences, stream errors for the failure runs, and the clean-close test asserting the stream ends with an OK status and
the process exits after `FirmwareApplying`.

```bash
cargo test -p bmc-mock --test upgrade_scenarios
```

The tests spawn the mock with `--fast-upgrades`, so the whole suite finishes in about a second.
