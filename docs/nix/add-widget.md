# Adding a New Widget to the Nix Build

This guide covers all the places you need to touch when adding a new widget to the build system. We use `my-widget` as
the example name throughout.

## Prerequisites

- The widget Rust crate already exists under `widgets/my-widget/` with a `Cargo.toml` whose package name is
  `bmc-widget-my-widget`.
- The crate is listed as a workspace member in the root `Cargo.toml`.

## 1. `nix/crates.nix` — Register the crate

Add an entry so the Nix build knows where to find the crate source and what Cargo package it maps to.

```nix
widget-my-widget = defineCrate {
  path = "./widgets/my-widget";
  packageName = "bmc-widget-my-widget";
};
```

| Field         | Description                                        |
| ------------- | -------------------------------------------------- |
| `path`        | Relative path from the repo root to the crate dir  |
| `packageName` | Must match the `name` in the widget's `Cargo.toml` |

## 2. `workspace.nix` — Add to the widgets attrset

The `widgets` attrset in `workspace.nix` drives the cartesian product that builds every widget for every arch/profile
combination. Add your widget here:

```nix
widgets = {
  # ...existing widgets...
  my-widget = {
    crate = bmc.crates.widget-my-widget;
    features = [ "standalone" ];
    runtimeDepsKind = "slint";  # or "native" for GPU widgets
  };
};
```

| Field             | Description                                                       |
| ----------------- | ----------------------------------------------------------------- |
| `crate`           | Reference to the crate defined in `nix/crates.nix`                |
| `features`        | Cargo features to enable (most widgets need `"standalone"`)       |
| `runtimeDepsKind` | `"slint"` for Slint/winit widgets, `"native"` for GPU/EGL widgets |

This single entry produces all per-arch packages (e.g. `widget-my-widget-armv7-glibc-release`) and includes the widget
in the combined `widgets` and `widgets-armv7-glibc-*` outputs.

## 3. `nix/packages.nix` — Define release metadata

Add a package definition with version and metadata used by the init index and upgrade system.

```nix
my-widget = {
  pkg = mkWidgetPackage {
    name = "my-widget";
    crate = crates.widget-my-widget;
    inherit profile;
    runtimeDeps = widgetRuntimeDeps.slint;  # or .native

    features = [ "standalone" ];
  };
  version = "1.0.0";
  category = "widget";
  description = "My widget description";
  upgrade_strategy = null;
  install_strategy = null;
};
```

| Field              | Description                                                      |
| ------------------ | ---------------------------------------------------------------- |
| `name`             | Widget name (used for directory layout under `lib/bmc-widgets/`) |
| `crate`            | Reference to the crate from `crates.nix`                         |
| `runtimeDeps`      | `widgetRuntimeDeps.slint` or `widgetRuntimeDeps.native`          |
| `features`         | Cargo features to enable                                         |
| `version`          | Semver version string for the package index                      |
| `category`         | Always `"widget"` for widgets                                    |
| `description`      | Human-readable description shown in the package index            |
| `upgrade_strategy` | How the device handles upgrades (`null` for widgets)             |
| `install_strategy` | How the device handles installation (`null` for widgets)         |

> **Renaming is not free.** The attribute name here (e.g. `my-widget`) is the package's identity in the index —
> `nix/init-artifacts.nix` copies it verbatim as the package `name`. There is no in-index rename migration. Renaming a
> widget that devices already carry as a system package (listed in `initPackageNames`) breaks upgrades on those devices:
> a device provisioned before the rename still lists the old name, `CheckForUpgrade` resolves it to `PackageNotFound` →
> `MissingSystemPackages`, and because a firmware upgrade also upgrades packages, that blocks firmware upgrades too.
> Rename such a widget only together with a fleet re-provision from a post-rename init tarball.

## 4. `nix/init-artifacts.nix` — Include in default device image (optional)

If the widget should be present on the Deck out of the box (i.e. included in the init tarball), add its name to the
`initPackageNames` list:

```nix
initPackageNames = [
  "core"
  "nix"
  "widget-flip-clock"
  "my-widget"        # <-- add here
];
```

The name must match the attribute name used in `nix/packages.nix`.

Skip this step if the widget will only be installed on-demand.

## Deploying to a device

Once the widget is registered in `nix/packages.nix`, you can deploy it to a device using `deck deploy`. It builds the
full Nix package, copies it to the device and activates a new profile generation with it. It works for both **native**
and **wasm** widgets:

```bash
nix run .#deck -- deploy --device 192.168.1.2 --packages '.#deck-packages.widget-flip-clock'
```

The script:

1. Builds the `deck-packages.widget-flip-clock.pkg` flake output
2. Copies the Nix closure to the device via `nix copy`
3. Builds and activates new generation of the bmc profile

Re-running for the same widget is safe — the symlink step uses `ln -sf` and overwrites existing entries. The script does
not garbage-collect stale files left over from previous deploys, so if a new build drops a file the previous one
shipped, the old symlink remains behind; clear the affected profile entries when the file layout changes.

### Fast iteration with `nix-cargo-deploy.sh` (native widgets only)

After the initial Nix deploy, use `nix-cargo-deploy.sh` for faster edit-compile-deploy cycles on **native widgets**. It
only uploads the binary without rebuilding the full Nix package:

```bash
nix develop .#armv7-glibc-release
./scripts/nix-cargo-deploy.sh widget my-widget 192.168.1.2
```

This requires the widget to already be present on the device (deployed at least once with `deck deploy` or via the init
tarball).

NOTE: any subsequent `deck deploy` will override the effect of `nix-cargo-deploy.sh`

**Wasm widgets are never deployed through `nix-cargo-deploy.sh`** — that script handles native binaries only. For wasm
widgets, re-run `deck deploy`.
