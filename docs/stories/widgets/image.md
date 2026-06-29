# Image Widget

The image widget displays a remote image — fetched from an operator-configured URL — fitted to the widget viewport. It
re-fetches on a configurable interval and caches the result on flash, so the picture survives restarts and stays on
screen while a refresh is in flight or the network is down.

## User stories

### Show a remote image

> As an operator, I want the Deck to display an image from a URL I choose, so I can show artwork, branding, or a
> dashboard snapshot.

- The URL points at a **PNG or JPEG**; the image is drawn fitted to the widget.
- The URL may contain `{{width}}` and `{{height}}` placeholders, substituted with the widget's pixel size so a
  cooperative server can return an already-sized image instead of a giant one.
- A *Refresh interval* (seconds) controls how often the image is re-fetched.
- While the first image is still loading, the widget shows a placeholder rather than a blank panel.

### Choose how the image fits

> As a user, I want to choose whether the whole image is shown or it fills the widget, so it looks right for the picture
> and the layout.

- A *Sizing* option selects **Fit — whole image** (letterboxed, the default) or **Fill — crop to edges** (the image
  covers the widget and the overflow is cropped).

### Keep the picture up while it refreshes or goes offline

> As a user, I want the current image to stay visible while a new one loads, so the widget never flashes to a blank or
> loading screen mid-cycle.

- The last good image is cached on flash and shown immediately on restart or when the widget returns from being
  off-screen — no re-fetch on that path.
- On a scheduled refresh the cached image stays on screen with a subtle *Updating* overlay, and swaps to the new image
  once it has loaded.
- If a fetch fails, the cached image remains rather than dropping to an error state; the fetch retries on its own.

## Constraints

- Inputs are limited to **PNG and JPEG**; other formats or oversized sources the server won't shrink are rejected to a
  placeholder/error state.
- URL, refresh interval, and sizing are manifest-driven widget parameters, configurable from the web UI.
- `{{width}}`/`{{height}}` templating is the primary strategy for large sources — the device avoids decoding a full-size
  image when the server honours the size hint.
- The cached image is one viewport-sized artifact per widget instance, stored under `/mnt/data/bmc/widget-cache/` and
  reclaimed by the host's asset-cache GC when the widget is removed.
