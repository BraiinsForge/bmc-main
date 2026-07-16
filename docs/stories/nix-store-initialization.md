# Nix Store Initialization

A device running firmware without Nix support gains its Nix package store — the storage all installable applications
live in — during an ordinary firmware upgrade. Initialization downloads the store contents from the Braiins factory
server, and the user expects it to be both seamless (no steps beyond the firmware upgrade itself) and secure (the device
installs only content published, and cryptographically signed, for exactly the firmware release being installed).

## User stories

### Gaining the store through an ordinary firmware upgrade

> As a user, I want upgrading to a Nix-capable firmware to set up the package store automatically so that I do not have
> to perform any extra steps.

- The regular firmware upgrade is the only action required: the upgrade prepares the device's data storage, downloads
  the initial store contents, and leaves the initial applications ready to activate on the next boot.
- The device downloads the store contents published for exactly the firmware version being installed — the factory
  server's release catalog maps each firmware version to its own artifacts, so the device never installs content the
  release was not tested with.
- If the release catalog has no entry for the firmware being installed, initialization fails visibly instead of guessing
  at a substitute.

### Downloading only trusted content

> As a user, I expect my device to download only verified files when it initializes its packages so that an attacker on
> the network path cannot make my device install tampered software.

- The release catalog is fetched from the factory server over TLS with certificate validation.
- Every published initialization tarball carries an Ed25519 signature, and the device verifies the downloaded bytes
  against the signing key provisioned in its factory configuration before anything is extracted.
- A catalog entry that offers no signature, or a factory trust anchor that is malformed, aborts initialization before
  the download even starts; a downloaded tarball that fails verification is deleted and never extracted.
- Verification is on by default. Development setups can disable it explicitly, and the device then warns loudly that it
  is trusting the transport alone.

### Recovering an inconsistent store

> As a user, I want a broken or half-initialized store to be replaced automatically at the next firmware upgrade so that
> a failed attempt never leaves my device stuck.

- A firmware upgrade that finds an absent, incomplete, or inconsistent store wipes it and reinitializes it to match the
  firmware being installed.
- An interrupted initialization is never mistaken for a completed one and restarts cleanly — see
  [Nix Store & Profile Power-Loss Safety](nix-store-durability.md).

## Constraints

- Initialization is carried by the first Nix-capable firmware release, which users cannot skip; the factory server must
  keep a catalog entry for every Nix-capable firmware version.
- Release publishing must sign every catalog entry before firmware carrying this initialization ships — the device
  refuses unsigned entries by default.
- Signature verification covers tarballs downloaded from the network; initializing from a locally supplied tarball is a
  development and recovery path that trusts the operator.
- Signatures defend the network path between the device and the download server; they do not defend against a
  compromised publisher — whoever holds the signing key defines what is authentic.
- The signature authenticates the tarball bytes, not the firmware-to-tarball mapping: any genuinely published tarball
  verifies, so an attacker who defeats TLS could pair the current catalog entry with a signed tarball from a different
  release. This matches the trust model of the firmware images themselves and is accepted; binding the firmware version
  into the signed data is possible future hardening.
- TLS certificate validation requires a roughly correct system clock.
