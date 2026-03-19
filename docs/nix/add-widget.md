# Adding a New Widget to the Nix Build

This guide covers all the places you need to touch when adding a new widget to the
build system. We use `my-widget` as the example name throughout.

## Prerequisites

- The widget Rust crate already exists under `widgets/my-widget/` with a
  `Cargo.toml` whose package name is `bmc-widget-my-widget`.
- The crate is listed as a workspace member in the root `Cargo.toml`.

## 1. `nix/crates.nix` — Register the crate

Add an entry so the Nix build knows where to find the crate source and what
Cargo package it maps to.

```nix
widget-my-widget = defineCrate {
  path = "./widgets/my-widget";
  packageName = "bmc-widget-my-widget";
};
```

| Field         | Description                                          |
|---------------|------------------------------------------------------|
| `path`        | Relative path from the repo root to the crate dir    |
| `packageName` | Must match the `name` in the widget's `Cargo.toml`   |

## 2. `workspace.nix` — Add to the widgets attrset

The `widgets` attrset in `workspace.nix` drives the cartesian product that
builds every widget for every arch/profile combination. Add your widget here:

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

| Field             | Description                                                        |
|-------------------|--------------------------------------------------------------------|
| `crate`           | Reference to the crate defined in `nix/crates.nix`                 |
| `features`        | Cargo features to enable (most widgets need `"standalone"`)        |
| `runtimeDepsKind` | `"slint"` for Slint/winit widgets, `"native"` for GPU/EGL widgets  |

This single entry produces all per-arch packages (e.g.
`widget-my-widget-armv7-glibc-release`) and includes the widget in the
combined `widgets` and `widgets-armv7-glibc-*` outputs.

## 3. `nix/packages.nix` — Define release metadata

Add a package definition with version and metadata used by the init index and
upgrade system.

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

| Field              | Description                                                        |
|--------------------|--------------------------------------------------------------------|
| `name`             | Widget name (used for directory layout under `lib/bmc-widgets/`)   |
| `crate`            | Reference to the crate from `crates.nix`                           |
| `runtimeDeps`      | `widgetRuntimeDeps.slint` or `widgetRuntimeDeps.native`            |
| `features`         | Cargo features to enable                                           |
| `version`          | Semver version string for the package index                        |
| `category`         | Always `"widget"` for widgets                                      |
| `description`      | Human-readable description shown in the package index              |
| `upgrade_strategy` | How the device handles upgrades (`null` for widgets)               |
| `install_strategy` | How the device handles installation (`null` for widgets)           |

## 4. `nix/init-artifacts.nix` — Include in default device image (optional)

If the widget should be present on the Deck out of the box (i.e. included in
the init tarball), add its name to the `initPackageNames` list:

```nix
initPackageNames = [
  "core"
  "nix"
  "digital-clock"
  "flip-clock"
  "my-widget"        # <-- add here
];
```

The name must match the attribute name used in `nix/packages.nix`.

Skip this step if the widget will only be installed on-demand.

## Deploying to a device

Once the widget is registered in `workspace.nix`, you can deploy it to a
device using `nix-deploy-new-widget.sh`. This script builds the full Nix
package and copies its closure to the device store:

```bash
./scripts/nix-deploy-new-widget.sh my-widget 192.168.1.2
# or
DEVICE_IP=192.168.1.2 ./scripts/nix-deploy-new-widget.sh my-widget
```

The script:
1. Builds the `widget-my-widget-armv7-glibc-release` flake output
2. Copies the Nix closure to the device via `nix copy`
3. Symlinks the widget files into `/run/current-profile/`

### Fast iteration with `nix-cargo-deploy.sh`

After the initial Nix deploy, use `nix-cargo-deploy.sh` for faster
edit-compile-deploy cycles. It only uploads the binary without rebuilding
the full Nix package:

```bash
nix develop .#armv7-glibc-release
./scripts/nix-cargo-deploy.sh widget my-widget 192.168.1.2
```

This requires the widget to already be present on the device (deployed
at least once with `nix-deploy-new-widget.sh` or via the init tarball).
