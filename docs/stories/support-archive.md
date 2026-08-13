# Support Archive

A one-click diagnostic bundle. When something goes wrong, the user downloads a single file from the device's web
interface and attaches it to their support request — no manual log gathering, no shell access, no guesswork about what
Braiins support will need.

## User stories

### One-click diagnostics

> As a user, I want to download one file with everything support needs so that I don't have to collect logs and settings
> by hand.

- The Settings page of the web UI offers a support archive download.
- The archive downloads as `support_archive__<timestamp>.zip`, so multiple archives from the same device stay
  distinguishable.
- Collection is best-effort: a diagnostic that cannot be gathered (missing file, failing command) is skipped and the
  rest of the archive is still produced. Requesting an archive never fails outright because one source is unavailable.

### Safe to share

> As a user, I want to hand the archive to support without exposing my secrets.

- Known credentials (Braiins Pool API keys, Wi-Fi passwords) are censored before files enter the archive — see
  [Support Archive Credential Censoring](support-archive-credential-censoring.md).
- The archive is a standard zip whose entries are password-protected (password `braiins`) so its contents are not
  casually readable in transit or in a ticket attachment. This is obfuscation for support workflows, not user-controlled
  secrecy — the censoring step is what protects credentials.

### System state snapshot

> As a support engineer, I want the device's runtime state at the moment of the report so that I can diagnose without
> asking the user to run commands.

- Output of standard system inspection commands: kernel log, process list, disk usage, environment, and firmware
  environment variables.
- Kernel and system information from `/proc` (memory, CPU, interrupts, mounts, modules, uptime, and similar).
- Device identity and versioning: BOS version, platform, board, and mode.
- System and application configuration files, including the device configuration and network setup.
- Application logs and the system log; the system log is captured after the other diagnostics run, so messages they
  trigger are included.

### Network diagnosis

> As a support engineer, I want enough network evidence to tell connectivity problems apart from device problems.

- Interface configuration and routing tables.
- A ping reachability report covering localhost, public DNS, and the Braiins services the device depends on — including
  both package download servers.
- The device's public IP as seen from outside.
- A short packet capture from every network interface taken during archive collection.

### Nix package state

> As a support engineer, I want to reconstruct what is installed on the device so that I can investigate upgrade and
> package issues without the device in hand.

- Every available profile generation manifest is included, so the package set of each generation can be reconstructed.
  The Nix store contents themselves are never bundled.
- A profile state summary records the generation numbers, the current target, any `next` / `next.*` upgrade targets,
  temporary profile entries, and whether each generation still has its manifest.
- The Nix database is included, showing which store paths the device considers registered and valid.
- The Nix upgrade configuration (package servers, garbage collection settings) and the Nix daemon configuration are
  included.

## Constraints

- The archive is assembled on demand.
- The download is a password-protected zip aimed at support tooling; it is not intended to be browsed by the user
  directly.
- Credential censoring covers known credential locations only — see the censoring story for the exact list.
- Nix store *contents* (binaries, libraries) are out of scope by design; the archive captures package *state* so it
  stays small enough to attach to a ticket.
