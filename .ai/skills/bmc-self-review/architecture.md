# Architecture checklist

## Logic in the owning layer

- **Effects beyond the config go through their owner, not `ConfigHandle`.** `ConfigHandle` changes and persists
  configuration. An operation that drives the device goes through the manager that owns the effect: `SystemManager` for
  brightness, sound, night mode and LEDs, `NetworkManager` for network settings, and the platform's `BmcManager` for
  reboot and factory reset. A change that is only a config write, whose effect rides on the notification
  `ConfigHandle::save` already sends, may call `ConfigHandle` directly.
- **Config saves don't roll back.** A failed save keeps the change in memory and reports the failure. See
  `docs/devel/config-persistence.md`.
- **Gate on capabilities, not products.** Hardware-dependent behaviour reads a `bmc_platform::HardwareProfile`
  capability. A comparison against a product enum or name breaks for the next product.
- **The core stays platform-neutral.** Product logic belongs in `bmc`, and hardware or OS specifics in `bmc-openwrt`,
  the driver crates and the `bmc-net` backends. The frontend renders what the backend decides and doesn't decide it
  again.
- **Model what is drawn.** Layout and hit-testing are computed from the geometry the code itself draws, never from the
  internals of a third-party renderer.

## Formats and compatibility

- **Wire and persisted formats are versioned.** A change to a persisted config or state file, the widget protocol or a
  Wayland protocol is either backward compatible or bumps the version. A widget manifest has no format version of its
  own (its `version` is the widget's), so a change to it stays loadable by older and newer releases of the BMC
  application: a new field is optional with a default. Protocol XML changes follow the `bmc-wayland-protocol-changes`
  skill. Widget protocol changes follow `docs/devel/wasm-host/protocol-versioning.md`.
- **The web gRPC API is internal.** `bmc-grpc/proto/web/` ships in the same image as the frontend, so it changes without
  a version (`docs/devel/frontend.md`). A change to an RPC updates its callers outside `frontend/` in the same branch,
  such as `bmc-tui` and `bmc-virt/relay`. Search for the service and method, since `bmc-tui` joins the package name on
  at run time: `rg '<Service>[/.]<Method>' --glob '!frontend/**' --glob '!bmc-grpc/**'`.
- **Two versions meet at an upgrade.** For a new variable, flag or file read across an upgrade, name which installed
  version writes it and which reads it. An older peer that doesn't know it gets a gate or a compatible default.
- **Persisted structs tolerate unknown keys.** Never use `#[serde(deny_unknown_fields)]` on them.
- **Widget package files are untrusted.** The one package file the device serves is the manifest icon. Its extension is
  checked against `ICON_EXTENSIONS` in `bmc-upgrade/src/widget_package.rs` (mirrored at build time by
  `nix/public-asset.nix`), and `handle_widget_icon` sends it with `nosniff` and `Content-Security-Policy: sandbox`,
  since an SVG opened directly runs its script on the device's origin. A branch that serves another package file sends
  the same headers and takes its type from an allowlist, never from the file.

## Device reality

- **Performance claims come from a Deck.** Frame time, memory or CPU claims are measured on the device (see the
  `bmc-deck-host-measurement` skill), not inferred from a desktop run.
- **Target viewports.** A rendering change is checked at the viewport sizes of the products it targets.

## Docs that drift

For each changed path, re-read the matching docs and fix whatever the diff made wrong:

| Changed paths                                   | Docs                                                                                              |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `bmc-wasm-host/`, `bmc-wasm-runtime/`           | `docs/devel/wasm-host/`                                                                           |
| `bmc-widget*/`, `widgets-wasm/`, `widgets/`     | `docs/devel/wasm-widgets/`, `docs/devel/widget-*.md`                                              |
| `system-overlays/`, `deck-*-v1/`                | `docs/devel/system-overlays/`                                                                     |
| `bmc-grpc/proto/`                               | `docs/devel/grpc/`, `docs/devel/frontend.md`                                                      |
| `bmc-upgrade/`, `bmc-nix/`                      | `docs/devel/upgrades.md`, `docs/devel/profiles.md`, `docs/devel/firmware-package-interlinking.md` |
| `bmc-platform/`                                 | `docs/devel/platforms.md`                                                                         |
| `bmc-render/`, the compositor in `bmc-openwrt/` | `docs/architecture/overview.md`, `docs/devel/display-scanout.md`                                  |
| `bmc-mock/`                                     | `docs/devel/mock/`                                                                                |
| anything user-visible                           | `docs/stories/`                                                                                   |

`docs/devel/README.md` indexes the rest. A new component that needs its own write-up gets an entry there.
