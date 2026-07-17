# Nix Integration — Detailed Implementation Sketch

File-level details for the Nix integration. For the high-level overview see
[nix-daemon-overview.md](nix-daemon-overview.md).

---

## New Crates

### `bmc-nix/` (library)

Described fully in [bmc-nix.md](bmc-nix.md). Key modules used by the rest of the system:

- `index::fetch_and_merge_indexes()` — called by `SystemUpgradeService` to check for Nix package updates
- `manifest::read_current_manifest()` / `compute_upgrade_plan()` — diffing installed vs available, building an upgrade
  plan (produces a ProfileGeneration for apply)
- `store::copy_store_paths()` — downloading packages from binary caches
- `store::init_store()` — used by the initializer for first-time setup
- `profile::build_profile()` / `activate_profile()` — profile construction and atomic switch
- `upgrade::apply_profile_change()` — apply a precomputed upgrade plan (copy → build → optional activate → gc)

### `bmc-nix-initializer/` (binary)

New crate, added to workspace `Cargo.toml` members and `workspace.nix` build targets (ARMv7 cross-compile).

```
bmc-nix-initializer/
├── Cargo.toml
├── ui/
│   └── initializer.slint       # Minimal Slint UI (status + progress bar)
└── src/
    └── main.rs                 # Entry point
```

**Dependencies:**

- `bmc-nix` — `store::init_store()`, types
- `bmc-display` — `DisplayDriver`, `DisplayController`, software renderer
- `bmc-shared/ii-net-drv` — `OpenwrtWifiManager` for WiFi scan/connect
- `reqwest` (rustls) — HTTP client for tarball download
- `tokio`, `tracing`

**Slint UI (`ui/initializer.slint`):** A single-window UI with a status label, a progress bar, and an optional secondary
label for details (e.g. SSID, error message). States driven from Rust via Slint property bindings — same pattern as
`bmc-display/ui/main.slint`.

**`src/main.rs` flow:**

01. Init logging, set up DRM display (reuse `linux_drm_platform` from `bmc-openwrt`)
02. Check if Nix store is initialized (e.g. check for profile gcroot sentinel)
03. If initialized → exit 0
04. Show "Checking system..." on display
05. Check network connectivity
06. If no WiFi:
    - Enter AP mode (via `OpenwrtWifiManager`)
    - Show AP SSID + "Connect to WiFi" on display
    - Start captive portal / listen for WiFi config (reuse `InitialSetup` WiFi pattern from `bmc/src/initial_setup.rs`)
    - On WiFi configured → connect, show "Connecting..."
07. Fetch the package feed from the factory server configured in `/etc/nix-upgrade/servers.json`
08. Match a feed entry to current BOS version from `/etc/bos-version`
09. If no matching entry found:
    - Show "Downloading" progress, "Upgrading system..."
    - Upgrade BOS using the current upgrade process (fetch firmware index, download, sysupgrade → reboot)
    - After reboot, the initializer runs again from step 1 with the new BOS version
    - In case there is no new version and no matching tarball, show error message to user to report. It should include
      the bos version and the available versions for factory.
10. Download tarball — show progress bar (downloaded MB / total MB)
11. Verify tarball signature against `known_public_key` from `servers.json`. If verification fails, delete the tarball
    and show a non-retryable error. TLS certificate validation remains enabled: initialization runs during sysupgrade,
    after the device has already downloaded the firmware over TLS, and therefore relies on the same system-clock and
    certificate-trust prerequisites. The tarball signature adds content authentication independently of the transport.
12. Extract tarball to `/nix/store` — show "Installing packages..."
13. Activate initial profile from tarball's `profile_path`
14. Exit 0, `bmc` starts normally

---

## Modified Files

### `Cargo.toml` (workspace root)

Add new members:

```toml
members = [
    # ... existing ...
    "bmc-nix",
    "bmc-nix-initializer",
]
```

### `workspace.nix`

Add `bmc-nix-initializer` as a build target alongside `bmc-openwrt`. It needs the ARMv7 cross-compile profile. The
initializer binary should be statically linked (musl).

---

### `bmc/src/system_upgrade.rs`

The core of the integration. `SystemUpgradeService` currently only handles BOS firmware. It needs a parallel Nix upgrade
path.

**Changes:**

1. **New field:** Add `nix_config` or similar holding Nix server config (servers.json path, profile dir, gc config) —
   passed in from `App::init()`.

2. **`check_for_upgrade()`** — extend to also call `bmc_nix::index::fetch_and_merge_indexes()` and
   `bmc_nix::manifest::compute_upgrade_plan()` using the current profile manifest plus requested additions/removals, and
   retain a `ProfileGeneration` for the apply step. Return a combined result covering both firmware and Nix package
   updates.

3. **New method `execute_upgrade()`** (or rename existing flow) — orchestrates the combined upgrade using a precomputed
   plan:

   - Apply upgrade plan (`bmc_nix::upgrade::apply_profile_change`)
   - If BOS upgrade needed: call existing `verify_and_upgrade()` (sysupgrade, triggers reboot)
   - After reboot (or immediately if Nix-only): optionally activate profile (`bmc_nix::profile::activate_profile`)
   - GC old generations (`bmc_nix::gc::cleanup_generations`)

4. **`SystemUpgradeState` enum** — add new variants:

   ```rust
   CopyingPackages { copied: u32, total: u32 },
   BuildingProfile,
   ActivatingProfile,
   ```

5. **`SystemUpgradeError` enum** — add Nix-specific variants:

   ```rust
   NixIndexFetchFailed(String),
   NixCopyFailed(String),
   NixProfileBuildFailed(String),
   NixActivationFailed(String),
   ```

6. **`autoupgrade_trigger()`** — replace firmware-only flow with combined flow.

---

### `bmc-grpc/proto/web/upgrade.proto`

Extend the gRPC contract to expose Nix upgrade information.

**New/modified messages:**

```protobuf
message PackageUpdate {
  string name = 1;
  string old_version = 2;
  string new_version = 3;
  string category = 4;
}

message CheckForUpgradeResponse {
  UpgradeMetadata latest_release = 1;           // existing (BOS)
  repeated ReleaseInfo previous_releases = 2;    // existing
  repeated PackageUpdate package_updates = 3;    // NEW: Nix packages with updates
  bool bos_upgrade_required = 4;                 // NEW: whether reboot needed
}
```

**Streaming progress** — extend `DownloadFirmwareResponse` (or add a new streaming RPC) to cover Nix phases:

```protobuf
message NixProgress {
  oneof phase {
    CopyingPackages copying = 1;
    BuildingProfile building = 2;
    ActivatingProfile activating = 3;
  }
}

message CopyingPackages {
  uint32 copied = 1;
  uint32 total = 2;
}
```

The exact proto shape should be refined during implementation — the key point is that the frontend needs visibility into
all phases.

---

### `bmc/src/web/grpc/upgrade_service.rs`

Wire the extended proto to `SystemUpgradeService`. Map the new combined check result into the extended
`CheckForUpgradeResponse`. Forward Nix progress states through the streaming RPC.

---

### `bmc/src/startup.rs`

**`App::init()`:**

- Accept Nix configuration (servers.json path, profile dir) — either as new fields in `Configuration` or a separate
  `NixConfig` struct.
- Pass Nix config into `SystemUpgradeService::new()`.

**`Configuration` struct:**

- Add fields:
  ```rust
  pub nix_servers_path: PathBuf,     // /etc/nix-upgrade/servers.json
  pub nix_profile_dir: PathBuf,      // /nix/var/nix/gcroots/profiles/bmc
  pub nix_gc_config_path: PathBuf,   // /etc/nix-upgrade/gc.json
  ```

---

### `bmc-openwrt/src/main.rs`

Pass Nix configuration paths to `Configuration` before calling `bmc::entry::main()`. The paths are well-known constants
on the device (`/etc/nix-upgrade/servers.json`, etc.).

---

### `bmc/src/manager.rs` — `BmcManager` trait

Likely **no changes** needed. BOS upgrade stays as `upgrade()`. Nix operations go through `bmc-nix` library functions
called directly by `SystemUpgradeService`. The manager trait stays focused on platform-specific operations.

If the combined upgrade flow needs to persist state across a BOS reboot (e.g. "activate Nix profile after reboot"), a
new method like `set_post_reboot_action()` could be added, or it could be handled via a file-based marker (similar to
the existing `check_and_remove_upgrade_marker()`).

---

### `bmc/src/display_tasks.rs`

Subscribe to the new `SystemUpgradeState` variants. Map them to Slint display screens:

- `CopyingPackages` → show "Downloading packages..." + count
- `BuildingProfile` → show "Building profile..." (this is likely going to be quick)
- `ActivatingProfile` → show "Activating..."

These are additions to the existing match on `SystemUpgradeState` in the display task's upgrade state handler.

---

### `bmc-display/ui/main.slint`

Add new screen definitions (or states within the existing upgrade screen) for the Nix phases. Minimal — a status label
and optionally a progress indicator.

---

### `bmc-mock/src/manager.rs`

No trait changes needed, but `bmc-mock` as a whole needs to simulate the Nix upgrade flow for frontend development.

### `bmc-mock/src/main.rs`

- Provide mock Nix index responses (mock HTTP server or hardcoded data)
- Simulate store copy / profile build delays in the `SystemUpgradeService` path
- Could use a `--with-nix-mock` flag or just always enable it

---

### Frontend

#### `bmc-grpc/proto/web/upgrade.proto` → `frontend/src/proto/gen/`

After proto changes, run `cd frontend && make gen` to regenerate TypeScript types.

#### `frontend/src/pages/workspace/Settings/components/SectionUpgrade/SectionUpgrade.tsx`

The `UpgradeFromFeedStatus` state machine needs new states for Nix phases. The component needs to:

- Display package update list from `CheckForUpgradeResponse.package_updates`
- Show whether reboot is required (`bos_upgrade_required`)
- Show multi-phase progress during the upgrade (copying → building → BOS upgrade → activation)
- Update the status text and progress indicators for each phase

#### `frontend/src/pages/workspace/Settings/components/SectionUpgrade/SectionUpgrade.scss`

Styling for new elements (package update list, multi-phase progress).

---

---

## Summary

| File / Crate                               | Change Type | What                                                        |
| ------------------------------------------ | ----------- | ----------------------------------------------------------- |
| `bmc-nix/`                                 | New crate   | Library — all Nix operations                                |
| `bmc-nix-initializer/`                     | New crate   | Boot-time initializer binary with Slint UI                  |
| `bmc-nix-initializer/ui/initializer.slint` | New file    | Minimal progress/status display                             |
| `Cargo.toml`                               | Modify      | Add workspace members                                       |
| `workspace.nix`                            | Modify      | Add build target for initializer                            |
| `bmc/src/system_upgrade.rs`                | Modify      | Combined BOS + Nix upgrade orchestration, new states/errors |
| `bmc/src/startup.rs`                       | Modify      | Accept and pass Nix config                                  |
| `bmc/src/display_tasks.rs`                 | Modify      | Handle new upgrade state variants                           |
| `bmc-grpc/proto/web/upgrade.proto`         | Modify      | Package updates, Nix progress messages                      |
| `bmc/src/web/grpc/upgrade_service.rs`      | Modify      | Wire new proto to service                                   |
| `bmc-openwrt/src/main.rs`                  | Modify      | Pass Nix config paths                                       |
| `bmc-mock/src/main.rs`                     | Modify      | Mock Nix index/upgrade flow                                 |
| `bmc-display/ui/main.slint`                | Modify      | New screens for Nix upgrade phases                          |
| `frontend/.../SectionUpgrade.tsx`          | Modify      | Multi-phase progress, package list                          |
| `frontend/.../SectionUpgrade.scss`         | Modify      | Styling for new elements                                    |
