# Config Migration on BMC Application Upgrade

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
- Each upgrade step translates the widgets it has an equivalent for and drops the rest. In the current v0 → v1 step that
  means the clock, block height, halving countdown, image, Braiins Pool, and ticker widgets keep their settings, plus
  the Braiins Forge remote widgets that now have a WASM equivalent — weather, nameday, ISS position, random facts, and
  SpaceX launch (matched by their URL to the WASM widget's ID). Their positions and user-configured settings carry over
  and they work immediately.
- The Braiins Pool widget also keeps its account: the pool account it used is bound to the new widget's account slot, so
  the user never re-enters the API key. A legacy pool widget that had no account arrives unbound and shows its bind
  prompt.
- The legacy ticker widgets all land on the two native ticker widgets. The built-in BTC ticker, the remote exchange-rate
  widget, and the remote sparkline and candlestick single tickers become the single ticker widget — the base and quote
  currencies collapse into its symbol parameter and the chart style carries over as its view. The remote ticker list
  becomes the native ticker list, with its usable symbols compacted into the leading symbol slots. Every legacy period
  maps to the closest supported window (`24h` becomes `1d`, longer windows clamp to `1mo`). An unusable period, or a
  symbol list with no usable entry at all, falls back to the shipped manifest default rather than dropping the widget.
- Any widget the step has no mapping for is dropped, with a `warn!` line naming the unsupported kind or URL. For v0 → v1
  this includes the blockchain-data widget and the remote Formula 1, NASA picture of the day, and debug widgets, none of
  which have a WASM counterpart. Dropped widgets are not preserved as empty placeholders.
- Saved accounts survive too. The version `1` → `2` step migrates a Braiins Pool account to the typed-credential shape,
  keeping its name and token; the user never re-enters it.
