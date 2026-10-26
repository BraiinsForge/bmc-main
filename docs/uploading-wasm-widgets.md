# Package a WASM widget for upload

The Deck's **Display Widgets → Install Widget** action accepts a Nix archive (`.nar`) containing one WASM widget. The
archive is a dump of the package root.

## Package layout

For a third-party widget named `my-widget` from `my-maintainer`, use this layout at the archive root:

```text
lib/
  bmc-widgets/
    my-maintainer/
      my-widget/
        manifest.json
        bin/
          my-widget
        lib/
          wasm/
            my-widget.wasm
          assets/                 # optional files read by the widget
        assets/                   # optional icon for the widget picker
          icon.svg
```

Use exactly one maintainer folder and one widget folder under it, with nothing beside `lib/` at the archive root. The
maintainer folder keeps third-party packages apart from official widget packages. Folder names consist of lowercase
letters, digits, and dashes.

The names in `bin/my-widget`, `lib/wasm/my-widget.wasm`, and the manifest's `binary` field must match the widget folder.
The launcher is the only executable file. The widget folder holds `manifest.json`, the launcher, the WASM module, and
files below `assets/` or `lib/assets/`.

## Manifest

Put `manifest.json` directly in the widget folder. For example:

```json
{
  "uid": "550e8400-e29b-41d4-a716-446655440000",
  "version": "1.0.0",
  "name": "My Widget",
  "description": "A sample widget for the Deck",
  "binary": "bin/my-widget",
  "icon": "assets/icon.svg",
  "supported_viewports": [
    {
      "type": "rectangular",
      "min_width": 1280,
      "max_width": 1280,
      "min_height": 480,
      "max_height": 480
    }
  ]
}
```

Generate your own version-4 UUID for `uid` and keep it when releasing new versions of the same widget. Use a semantic
version and declare the viewports the widget supports. The icon is optional and lives under `assets/`; the widget's
runtime assets, if any, go under `lib/assets/`.

See the [manifest specification](devel/widget-manifest.md) and its
[JSON Schema](../bmc-widget-manifest/manifest.schema.json) for all fields and validation rules. The
[official widget manifests](../widgets-wasm/) provide examples.

## Launcher script

`bin/my-widget` is this script with `@name@` replaced by `my-widget` and `@sdkMajor@` replaced by the SDK major the
device supports. For 26.10, that major is `0`.

```sh
#!/bin/sh
exec /run/current-profile/bin/bmc-wasm-thin-v@sdkMajor@ \
    --wasm "${0%/*}/../lib/wasm/@name@.wasm" \
    --asset-root "${0%/*}/../lib/assets" \
    "$@"
```

Make the script executable. Place the manifest, compiled `my-widget.wasm`, and optional icon at the paths shown above
before dumping the archive. Build the module with the matching [widget SDK](../bmc-wasm-runtime/README.md).

## Create and install the archive

After assembling the package root and its files, dump the directory that holds `lib/` as a NAR:

```sh
nix-store --dump path/to/package-root > my-widget.nar
```

On the Deck, open **Display Widgets**, choose **Install Widget**, accept the third-party notice, and select the `.nar`
file. To update, keep the same `uid` and increase `version`.
