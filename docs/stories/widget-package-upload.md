# Widget Package Upload

A widget built outside the official package feed can be installed on the Deck by uploading its package file from the web
UI. The device checks that the archive is a well-formed widget package before anything is installed, so a rejected file
leaves the device as it was. What an accepted widget does once it runs is not checked by Braiins Forge; the user accepts
that risk before uploading.

## User stories

### Install a widget from a file

> As a Deck user, I want to upload a widget `.nar` file in the Display Widgets tab so I can install a WASM widget
> without developer tooling or a package feed.

- The Display Widgets page has an **Install Widget** button, first among the page controls, that opens a file picker for
  a `.nar` file.
- Before the file picker opens, a warning says that packages installed from a file are not checked by Braiins Forge and
  may be malicious; the user accepts the risk or cancels. The same dialog says what to upload, the widget as a `.nar`
  archive of its package, and links to the BraiinsForge/bmc-main repository for instructions.
- The upload runs in the browser session the user is already signed in to; no extra login or token is needed.
- While the file uploads and installs, the button shows a spinner and cannot be pressed again.
- After a successful install the widget appears in the widget picker straight away, with no device restart.

### Know what happened

> As a Deck user, I want to see whether the installation succeeded or failed, and why it failed, so I know what to do
> next.

- A success message names the widget and its version and says whether it was newly installed, upgraded from a previous
  version, reinstalled (the same version rebuilt with different contents), or already installed as uploaded.
- A failure message carries the device's reason: the archive is malformed, is not a widget package, is larger than the
  limit, would downgrade the installed widget, would hide an installed widget or be hidden by one, arrived while another
  install or a software upgrade was running, does not fit on the device, or did not finish within the five-minute upload
  deadline.

### Trust the device to check the package

> As a Deck user, I want the device to inspect what I upload so a file that is not a WASM widget package is refused
> instead of installed.

- The page only offers `.nar` files, but the device examines the archive contents; the file name and extension decide
  nothing there.
- The checks cover the package's shape only. An accepted widget's behaviour is not reviewed and may be malicious, which
  is why the upload asks the user to accept the risk first.
- A package is accepted only when it holds exactly one WASM widget in the standard package layout: the widget's
  metadata, its WASM module, the launcher that starts it and, optionally, the widget's own assets.
- The widget may sit under a maintainer folder. Its package is then named after the maintainer and the widget, so a
  third-party widget does not take the place of a feed widget of the same name at the next upgrade.
- A maintainer folder named like an installed widget, or a widget named like an installed maintainer folder, is refused:
  the device would show only one of them.
- Archives with malformed or missing widget metadata, native (non-WASM) widget binaries, or any content outside that
  layout are rejected.

### Keep the device safe on failure

> As a Deck user, I want a rejected or failed upload to leave my device exactly as it was.

- Until the package passes every check it is not part of the active profile, and a rejected package is removed from the
  device again.
- A failed upload leaves the active profile and every previously installed widget unchanged. In the rare case that
  undoing a failed install fails as well, the device reports the error and the new widget may stay active.
- Installing a newer version of an already installed widget replaces it in place.

## Constraints

- One upload holds one widget package. Archives larger than 64 MiB are refused.
- Uploading an older version than the one installed is refused, as is a version longer than 64 characters.
- A widget uploaded without a maintainer folder replaces the feed widget of the same name only while its version is
  above the feed's. At the feed's version or below, the next upgrade, automatic ones included, puts the feed's build
  back, so a fix tried this way needs a version above the feed's.
- The upload waits for no running software upgrade: while one is in progress the upload is refused and can be retried
  afterwards.
- On a miner whose software is managed by BOSer, widget packages come from the managed upgrade flow: the Display Widgets
  page shows no Install Widget button and the device refuses an upload.
- Uninstalling a widget, a widget marketplace and publishing to a package feed are out of scope.
