# Nix Store Initialization

A device running firmware without Nix support gains its Nix package store — the storage all installable applications
live in — during an ordinary firmware upgrade. Initialization downloads the store contents from the Braiins factory
server, and the user expects it to be both seamless (no steps beyond the firmware upgrade itself) and secure (the device
installs only content published, and cryptographically signed, for the selected firmware or shared release entry).

## User stories

### Gaining the store through an ordinary firmware upgrade

> As a user, I want upgrading to a Nix-capable firmware to set up the package store automatically so that I do not have
> to perform any extra steps.

- The regular firmware upgrade is the only action required: the upgrade prepares the device's data storage, downloads
  the initial store contents, and leaves the initial applications ready to activate on the next boot.
- The device prefers store contents published for the exact firmware version. If no exact entry exists, it uses a
  published shared release entry with the same release version, variant and suffix, such as `26.09-plus-nightly`.
  Initialization, reset and package upgrades use the same selection rules.
- If neither entry exists, initialization fails visibly. An exact entry with broken or missing artifacts still fails; it
  never silently switches to shared content.

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
  publish an exact or matching shared catalog entry for each firmware it supports. Older clients require exact entries.
- Publishers choose whether to provide shared release entries. Shared development entries support the latest firmware;
  updating them may break older development builds. The device does not infer compatibility or distinguish production
  firmware to decide whether fallback is allowed.
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
