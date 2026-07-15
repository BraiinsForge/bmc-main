# Activation marker quarantine — design

Ticket: BDK-567. Never commit this spec unless the user explicitly asks.

## Goal

Keep boot activation recoverable when `current` or the matching `next.<bos-version>` path exists but is not a symlink.
Preserve the invalid filesystem entry for inspection, then use the same fallback behavior as an absent marker.

## Behavior

- A valid `current` symlink keeps its existing behavior.
- A regular file or directory at `current` is renamed to `current.invalid.<unix-timestamp-nanoseconds>`. Activation then
  selects the highest valid `<N>-link`, exactly as it does when `current` is absent. If no generation exists, activation
  is skipped.
- A valid matching `next.<bos-version>` symlink keeps its existing behavior.
- A regular file or directory at the matching `next.<bos-version>` path is renamed to
  `next.<bos-version>.invalid.<unix-timestamp-nanoseconds>`. `--generation next` then delegates to `current`, exactly as
  it does when the matching marker is absent.
- Non-symlink markers for other firmware versions retain the existing sweep behavior and are ignored. This change only
  quarantines an invalid marker selected for the running firmware.
- Dangling symlinks retain their existing fallbacks and are not quarantined.
- Permission failures and other errors that prevent inspection, quarantine, or directory synchronization remain hard
  activation errors.

If the timestamped destination already exists, append an incrementing numeric suffix rather than overwrite preserved
evidence. The profile lock serializes activation and upgrade writers; non-cooperating external mutation is outside this
recovery contract. The quarantine rename occurs while the lock is held. Fsync the profile directory after the rename so
the recovery state survives a crash.

## Implementation shape

Add one focused helper in `bmc-nix/src/activation.rs` that classifies a marker with `symlink_metadata()`:

- missing returns absent;
- symlink returns present without mutation;
- any other filesystem object is atomically renamed to a unique timestamped quarantine name, followed by a profile
  directory fsync, then returns absent.

Use the helper before resolving `current` and before evaluating the matching `next.<bos-version>`. Keep target
validation, entrypoint fallback, activation, revert, and marker-consumption logic unchanged.

## Error handling

Report quarantine failures with the source and destination paths so the boot log identifies the obstructing entry. Do
not silently remove an invalid entry or overwrite a previous quarantine. Do not reinterpret unrelated I/O errors as
absence.

## Testing

Drive the behavior through `bmc-nix-cli activate` integration tests:

- regular-file `current` is quarantined and latest is activated;
- directory `current` follows the same recovery path;
- regular-file matching `next.<bos-version>` is quarantined and current is activated;
- directory matching `next.<bos-version>` follows the same recovery path;
- quarantine names contain the original marker name and a numeric timestamp, and the original contents remain intact.

Run the focused activation tests during the red/green loop, then run `just validate` and require its final
`validate: OK` marker.
