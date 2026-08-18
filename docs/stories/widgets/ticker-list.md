# Ticker List Widget

The ticker list widget shows several financial instruments at once — stocks, indices, currency pairs, or cryptocurrency
pairs — one per row. Each row carries the symbol, the company or instrument name, a sparkline over the selected time
period, the current price, and a percentage change badge. Prices come from the Braiins Forge Nexus, and every row is
fetched independently so one bad symbol never blanks the others.

## User stories

### Watch several instruments at once

> As a user, I want a single widget that lists the prices I follow so I do not need one widget per symbol.

- Up to eight symbols are configured through the *Symbol 1* … *Symbol 8* parameters, which default to `NVDA`, `AAPL`,
  `TSLA`, `MSTR`, `JPM`, `META`, `SPY`, and `NFLX`.
- Each row shows the symbol, the instrument's name beneath it, the current price, and a signed percentage change.
- Leaving a slot empty skips it and the remaining symbols move up, so a blank slot never leaves a gap in the list.
- Clearing every slot shows `No symbols provided`.
- Supported examples include the stock `AAPL`, the S&P 500 index `^GSPC`, the currency pair `EUR-USD`, and the
  cryptocurrency pair `BTC-USD`. Availability varies by instrument; other symbol forms may not be supported.

### See as many rows as the tile can hold

> As a user, I want the list to adapt to the widget size so it stays readable in a small tile and uses the space in a
> fullscreen scene.

- The `full` size shows eight rows in two columns, filled left to right so the first two symbols share the top row.
- The `large` size shows four rows, and the `medium` and `small` sizes show two, all in a single column.
- Symbols configured beyond what the current size can show are neither fetched nor displayed.
- The `small` size drops the sparkline to leave room for the numbers, and shortens the instrument name budget.
- Instrument names longer than the row allows are truncated with an ellipsis.

### Read the trend for each row

> As a user, I want to see at a glance which of my instruments are up and which are down.

- The change badge is always signed and carries one decimal, for example `+5.3%` or `-2.8%`, measured from the opening
  price of the selected period to the latest price.
- A non-negative change is green and a negative change is red; the sparkline and its fill take the same colour.
- The sparkline traces the price across the whole selected period within its row.
- The price is formatted with as many decimals as its magnitude warrants — none at 100 000 and above, two at 1 and
  above, and progressively more for small values.
- Currency pairs made of two fiat currencies below a rate of 1000 use five decimals, or three when quoted in JPY.
- Number grouping and the decimal separator follow the device localization setting.

### Choose the time period

> As a user, I want to choose how far back the rows reach so the list reflects the horizon I care about.

- The *Time Period* parameter offers *1 Hour*, *1 Day*, *7 Days*, and *1 Month*, defaulting to *7 Days*.
- The period applies to every row at once and sets both the sparkline window and the percentage change.
- The chart resolution follows the period automatically, so the sparkline stays meaningful at every window.
- Changing the period reloads the prices but keeps the instrument names already fetched.

### Keep the list useful when one symbol fails

> As a user, I want a mistyped or unavailable symbol to affect only its own row.

- Each row fetches on its own; a failure degrades that row alone and leaves the rest live.
- A row still waiting for its first price reads `Loading…`.
- A symbol the data service does not recognize shows the typed symbol in red with `Not found` and `N/A` in place of the
  price while the row keeps polling normally.
- A recognized symbol whose selected period carries no bars reads `Closed` when its market is shut and `No data`
  otherwise.
- Any other failure shows `Unavailable` with `N/A` and keeps retrying on its own.
- If a row already has a price and the refresh starts failing, the last known price and sparkline stay on screen; after
  about seven and a half minutes the row gains a warning badge with the age of the last refresh. The `small` size shows
  the warning icon without the age.

### Know when a market is closed

> As a user, I want to know that a price is not moving because the market is shut rather than because the widget is
> stuck.

- When the data reports that an instrument's market is closed, the row shows a small marker after the symbol and its
  sparkline turns gray.
- Indices are never marked this way.

## Constraints

- The widget renders at the shared `small`, `medium`, `large`, and `full` sizes on rectangular viewports from 317x238 up
  to 1280x480. The round BFM100 face is not supported.
- Non-canonical rectangular viewports keep the closest shared size classification and scale by the widget fit factor, so
  BMM101's 480x320 fullscreen viewport shows the four-row Large layout shrunk to fit. Row and column counts, the
  sparkline toggle, and the name budget are layout structure and do not scale.
- *Symbol 1* … *Symbol 8* and *Time Period* are manifest-driven widget parameters, configurable from the web UI. The
  manifest format has no list type, so the symbols are eight separate string parameters rather than one array.
- The manifest subscribes to the device `localization` setting. The percentage change is deliberately not localized.
- Price data comes from `https://nexus.braiinsforge.com/api/v1/data/prices/`; each row polls every 300 seconds with a 15
  second per-request timeout, sized above the Nexus cold long-poll. A response that cannot be parsed is retried after 10
  seconds; a 400, 404, or 503 uses the normal 300-second cadence.
- Instrument names and market state come from `https://nexus.braiinsforge.com/api/v1/data/reference/` and are fetched
  best-effort. A row with no name available simply shows none and tries again after its next successful price refresh.
- Configuring fewer symbols than the current size can hold leaves the remaining rows blank.
- The widget has a fixed dark palette and does not recolour for night mode.
