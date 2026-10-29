# Config Migration on Upgrade

The device config carries a schema version. Whenever a BMC Application upgrade ships a newer config schema, the config
already on disk is migrated up to it automatically — once, on first start of the new version, with no prompt and no
manual step. The user never has to know that the schema changed, or that a file moved.

Migration is a chain of one-hop upgrade steps: each schema version knows how to read the version directly below it and
produce the next one. A config that is several versions behind is walked up one step at a time until it matches the
version the running BMC Application expects. Today the chain reaches schema version `2`: version `0` is the legacy
slint-monolith config, version `1` the first manifest-driven widget schema, and version `2` moves saved accounts to
typed credential instances. Every later schema bump adds one more step to the same chain, and the guarantees below
(automatic, transparent, and validated before write) hold for every future upgrade the same way.

## User stories

### Transparent upgrade

> As a user, I want my scenes and widget layouts to survive a BMC Application upgrade without having to re-create them.

- The migration runs once on first boot of the new BMC Application. No prompt, no manual step.
- The upgrade is applied in memory; the config on disk is left unchanged until the first time a setting is actually
  changed, at which point the upgraded config is written.
- Device settings survive too: alarms, night mode, brightness, sound volume, localization, scene cycling, the LED and
  boot-sound switches, and auto-upgrade preferences all carry over unchanged.
- Scene IDs, widget positions, and widget sizes are preserved. The grid the user built still looks the same.
- The v0 migration keeps at most 56 widgets active. When an enabled scene would cross that limit, the whole scene is
  retained but disabled; its widgets and settings remain available to the user.
- Migration continues examining later scenes after disabling an overflowing scene. A later scene remains enabled when
  all of its widgets fit within the remaining capacity.
- Each upgrade step translates the widgets it has an equivalent for and drops the rest. In the current v0 → v1 step that
  means the clock, block height, halving countdown, image, Braiins Pool, and ticker widgets keep their settings. The
  blockchain-data widget becomes the Bitcoin mining data widget, which has no settings of its own. The Braiins Forge
  remote widgets that now have a WASM equivalent come across as well — weather, nameday, ISS position, random facts,
  SpaceX launch, Formula 1, and NASA picture of the day (matched by their URL to the WASM widget's ID). Their positions
  carry over and they work immediately, and so do their settings wherever the WASM widget has somewhere to put them.
  Picture of the day arrives on the default source with the title shown, because the legacy widget carried nothing the
  WASM widget recognises; the SpaceX widget's seconds toggle drops for the same reason.
- The Braiins Pool widget also keeps its account: the pool account it used is bound to the new widget's account slot, so
  the user never re-enters the API key. A legacy pool widget that had no account arrives unbound and shows its bind
  prompt.
- The legacy ticker widgets all land on the two native ticker widgets. The built-in BTC ticker, the remote exchange-rate
  widget, and the remote sparkline and candlestick single tickers become the single ticker widget — the base and quote
  currencies collapse into its symbol parameter and the chart style carries over as its view. The remote ticker list
  becomes the native ticker list, with its usable symbols compacted into the leading symbol slots. Every legacy period
  maps to the closest supported window (`24h` becomes `1d`, longer windows clamp to `1mo`). An unusable period, or a
  symbol list with no usable entry at all, falls back to the shipped manifest default rather than dropping the widget.
- Any widget the step has no mapping for is dropped, with a `warn!` line naming the unsupported kind or URL. Every
  widget kind and every Braiins Forge slug the v0 → v1 step knows about now has a WASM counterpart, so what drops is a
  widget this firmware never shipped: an unrecognised kind, or a remote widget whose URL is not a Braiins Forge slug the
  step knows. Dropped widgets are not preserved as empty placeholders.
- Saved accounts survive too. The version `1` → `2` step migrates a Braiins Pool account to the typed-credential shape,
  keeping its name and token; the user never re-enters it.

### Widget settings across a widget upgrade

> As a user, I want a widget's settings to carry over when a widget release changes what the widget can be configured
> with, so I do not have to set up my scenes again.

- Each configured widget remembers which widget version its settings were saved for.
- Settings saved for an older version are converted before the widget starts, at boot and after a widget upgrade.
- When the conversion succeeds, the widget never runs on settings it no longer understands.
- A widget whose settings changed shape ships its own conversion. The ticker list turns its eight symbol fields into a
  list of rows, in the same order.
- A widget without a conversion keeps every setting it still knows and gets defaults for new ones. If a saved setting no
  longer fits, it starts on its saved settings as they are. Its saved settings change only when it is edited.
- Downgrading a widget leaves its settings as the newer version wrote them, so upgrading again finds them intact.
- A running widget whose own package did not change picks up its converted settings without restarting.
- While an upgrade applies, a scene or widget edit from the web UI waits for it to finish:
  - after eight seconds the edit is refused as temporarily unavailable;
  - retrying succeeds once the upgrade has applied.
- A conversion can fail, run longer than five seconds, or produce settings the widget refuses. Then:
  - the stored settings stay as they were and the device logs why;
  - the widget starts on them;
  - the conversion is tried again at the next boot or upgrade.
