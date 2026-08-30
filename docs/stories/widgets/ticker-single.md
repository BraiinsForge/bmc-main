# Single Ticker Widget

The single ticker widget follows one financial instrument — a stock, an index, a currency pair, or a cryptocurrency
pair. It shows a header with the instrument icon, its symbol, the selected time period, and a percentage change badge,
and fills the rest of the tile with either a sparkline behind a large price readout or a full candlestick chart with a
price axis, a volume strip, and time labels. Price history comes from the Braiins Forge Nexus.

## User stories

### Follow a single instrument

> As a user, I want to pick one symbol and see its current price so I can keep an eye on the thing I care about.

- The *Symbol* parameter is a free-form financial symbol; it defaults to `BTC-USD`.
- Supported examples include the stock `AAPL`, the S&P 500 index `^GSPC`, the currency pair `EUR-USD`, and the
  cryptocurrency pair `BTC-USD`. Availability varies by instrument; other symbol forms may not be supported.
- The header shows the base symbol in full brightness and the quote currency dimmed next to it. For a plain code the
  quote currency comes from the data itself.
- While no price has arrived yet the widget reads `Loading…`.
- Leaving the symbol empty shows `Enter symbol` and issues no request.

### Choose the time period

> As a user, I want to choose how far back the chart reaches so I can inspect short- and medium-term changes.

- The *Time Period* parameter offers *1 Hour*, *1 Day*, *7 Days*, and *1 Month*, defaulting to *7 Days*.
- The period sets both the chart window and the percentage change, which is measured from the first bar's opening price
  to the latest price.
- The period is printed in the header at the `medium`, `large`, and `full` sizes. The `small` size omits it to keep room
  for the price.
- The chart resolution follows the period automatically: 1 minute, 15 minutes, 1 hour, and 1 day respectively.

### Read the price change at a glance

> As a user, I want the direction of the move to be obvious without reading the numbers.

- The change badge is always signed and carries one decimal, for example `+5.3%` or `-2.8%`.
- A non-negative change is green and a negative change is red; the sparkline and its fill take the same colour.
- The price itself is formatted with as many decimals as its magnitude warrants — none at 100 000 and above, two at 1
  and above, and progressively more for small values, down to `<0.000001` for anything smaller than that.
- Currency pairs made of two fiat currencies below a rate of 1000 use five decimals, or three when quoted in JPY.
- Number grouping and the decimal separator follow the device localization setting.

### Switch between a sparkline and a candlestick chart

> As a user, I want to choose between a clean price trend and a detailed trading chart depending on what the widget is
> for.

- The *View* parameter is either *Sparkline* (the default) or *Candlestick*.
- The sparkline view draws a filled trend line across the lower part of the tile with the current price large and
  centred over it.
- The candlestick view draws individual bars with wicks, a dashed price grid, a price axis with the current price
  highlighted in a coloured badge, and a volume strip below the bars.
- Each candle is green when it closed at or above its open and red when it closed below, independently of the overall
  period trend.
- The volume strip only appears when the data carries volumes; without them the chart uses the full height.

### Read the candlestick time axis

> As a user, I want to know which part of the chart I am looking at.

- Time labels run under the candlestick chart at the `large` and `full` sizes. The `medium` and `small` sizes omit them
  because there is no room.
- The label granularity follows the period: clock times for *1 Hour* and *1 Day*, and day plus month for *7 Days* and *1
  Month*.
- Labels sit on day, month, or year boundaries and thin themselves out so they never collide or run off the left edge.
- Times follow the device timezone, and the clock and date formats follow the device settings.

### Know when a market is closed or the data is old

> As a user, I want to be able to tell stale numbers from live ones so I do not act on an old price.

- When the data reports that the instrument's market is closed, a pause marker covers its icon and its chart dims while
  the text stays legible. Indices are never marked or dimmed this way.
- When a refresh has been failing for longer than 90 seconds, a *last refresh* pill appears in the corner while the last
  known chart stays on screen.
- If the symbol is not recognized the widget shows `Symbol {symbol} not found` and keeps polling normally.
- If the symbol is recognized but the selected period carries no bars, the widget reads `{symbol} — market closed` when
  the market is shut and `No data for this period` otherwise.
- Any other failure with nothing on screen yet reads `{symbol} unavailable`; if a chart is already drawn it stays, and
  the widget keeps retrying on its own.

## Constraints

- The widget renders at the shared `small`, `medium`, `large`, and `full` sizes on rectangular viewports from 317x238 up
  to 1280x480. The round BFM100 face is not supported.
- Non-canonical rectangular viewports keep the closest shared size classification and scale by the widget fit factor, so
  BMM101's 480x320 fullscreen viewport shrinks rather than overflows.
- *Symbol*, *Time Period*, and *View* are manifest-driven widget parameters, configurable from the web UI.
- The manifest subscribes to the device `localization` setting. The candlestick time axis additionally follows the
  device timezone, time format, and date format.
- Price data comes from `https://nexus.braiinsforge.com/api/v1/data/prices/`; the widget polls every 60 seconds with a
  15 second per-request timeout, sized above the Nexus cold long-poll. A response that arrives but cannot be parsed is
  retried after 10 seconds; a 400, 404, or 503 uses the normal 60-second cadence.
- Market state comes from `https://nexus.braiinsforge.com/api/v1/data/reference/` and refreshes every 30 minutes.
- Both views use the period's supported resolution and merge adjacent bars when the plot is too narrow to draw them at
  least four pixels apart. Roughly 45 bars fit at `small` and 286 at `full`.
- Changing the symbol, period, or view clears the chart to `Loading…` and restarts the fetch.
- The widget has a fixed dark palette and does not recolour for night mode.
