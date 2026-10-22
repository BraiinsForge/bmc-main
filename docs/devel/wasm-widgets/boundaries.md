# WASM Widget Boundaries

A device runs several widgets across several scenes, all set up by the person using it. A widget changes only what
belongs to it: its own scene, state and data sources. These rules apply to every widget, official or not, and we check
them before listing a third-party widget or accepting a contribution. [Best Practices](best-practices.md) covers how to
build well inside them.

A **must not** is the platform contract, and the device enforces part of it. A **should not** is etiquette for sharing
the device.

## Must Not

**Must not: change device settings, scenes or configuration.**

- Brightness, night mode, alarms, the scene list and the widget's own params belong to the user. A widget that moves
  them overrides the person who set them and changes the conditions every other widget runs under.
- The SDK has no call for any of this. The only route is the internal API below.
- Instead: keep state inside the widget. A companion app talks to the widget through `http_listen(0, …)`. The host picks
  a free port, since a fixed one fails when something else holds it, and `listener.port()` then reads `0`. The port
  changes across restarts, so advertise it with `mdns` and have the app find the widget there. Anyone on the LAN can
  reach the listener, so authenticate the app with a pairing code shown on the screen.

**Must not: ship anything but a WASM widget.**

- The upload refuses a native executable ("is a native widget binary; only WASM widgets can be installed"), any file
  "not part of a WASM widget package", and any symlink.
- Daemons, `rc.local` lines, cron entries, CGI or shell scripts put on the device by another route run as root with no
  sandbox. Nothing upgrades them, removes them on factory reset or accounts for their memory, and a control port they
  open on loopback is reachable by every other widget.
- The Wayland `deck_widget` binary in [Widget Manifest](../widget-manifest.md) is the host's internal contract. The
  packaging generates it for a WASM widget; an author never writes one.
- Instead: keep everything the widget does inside the WASM package. If it needs something the SDK cannot do, open an
  issue describing the need.

**Must not: self-update by overwriting installed files or killing processes.**

- An installed widget lives in the Nix store under a profile the device manages. A copied-over wasm fails or falls out
  of step with the profile, and killing the host process is a widget crash.
- A download from a branch tip with no signature or hash check is a supply-chain hole on the user's device.
- Instead: publish each version as a new `.nar`. Uploading it upgrades the installed widget in place: same uid, higher
  version.

**Must not: use the BMC application's internal interfaces.**

- The gRPC-Web API, from the LAN or from loopback, serves the application's own web frontend. It changes between
  releases without notice, and on a device with a password the calls fail.
- On-device files and tools (`/etc/bmc/config.json`, `/mnt/data`, Nix store paths, `bmc-nix-cli`, `uci`) are
  release-internal and move without notice. A WASM widget has no filesystem access, so this covers the scripts and
  instructions shipped around it.
- Instead: use the SDK host imports, such as `system`, `led`, `credentials` and the network calls. If something is
  missing, open an issue describing the need.

**Must not: put a secret in a param.**

- Params are stored in plaintext in the device configuration and shown in the web UI.
- Instead: declare a credential slot. The host substitutes the secret into outbound fetch requests and WebSockets, so
  the widget never holds it; see [Credentials](credentials.md). HTTP Digest cannot use a slot yet.

## Should Not

**Should not: run ambient LED effects on the global tier.**

- The global tier plays on every scene that has no effect of its own, so a lasting global effect owns the strip across
  the whole device. Re-arming a timed one from a loop is the same thing.
- The global tier is for a short notification the user must see on any scene: a block found, a miner down.
  [Widget LED Effects](../../stories/widget-led-effects.md) describes the arbitration.
- Instead: `led::set_effect(effect, color, period, None)` for an endless scene-local effect, `Some(duration)` for
  pulses, and `led::stop()` when the widget has nothing to show.

**Should not: give install instructions beyond the web UI upload.**

- No ssh as root, config edits, `nix-store --add`, `bmc-nix-cli` or application restarts. Each is an internal interface,
  and a hand-edited config skips the validation the application applies to its own writes.
- Registering a package server has no web UI either, so an install guide cannot ask for it.
- Instead: point users at the Display Widgets upload, described in
  [Widget Package Upload](../../stories/widget-package-upload.md).

**Should not: hard-code one viewport.**

- A widget whose manifest supports one rectangle can only be placed full-screen on one product.
- Instead: declare `supported_viewports` and lay out from `widget_viewport()`, as
  [Display Geometry](display-geometry.md) describes.

**Should not: poll aggressively or hold many connections.**

- CPU, memory and network are shared by every widget on every scene, on screen or not.
- Instead: poll one host at a time, spread the polls over the refresh period with a minimum gap, and give up on an
  unreachable host after a second. `widgets-wasm/fleet-management` does all four.
  [Work sequentially](best-practices.md#work-sequentially) lists the runtime's limits.

**Should not: request frames faster than the content changes.**

- Every `request_frame_after` is a full wasm run, and the host already replays declared transitions at its own cadence.
- Instead: request a frame when the data changes, and run periodic work on a [timer](timers.md) started in `on_wake`.

**Should not: bundle unrelated features into one widget.**

- A mining widget with a weather screensaver brings a second data source, more params and the user's location, none of
  which the user asked for. Official widgets already do those jobs.
- Instead: ship a second widget. The user composes scenes.
