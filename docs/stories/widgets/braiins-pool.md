# Braiins Pool Widget

The Braiins Pool widget shows live hashrate, worker, and payout stats for one Braiins Pool (FPPS) mining account. It
reads the pool's public API with an account API key the user binds once as a saved account, and offers two scene styles:
an **Overview** of payout and worker stats, or a **Big Chart** filled by the hashrate history. Both styles render at all
four widget sizes, each size showing as much of the picture as its frame holds.

## User stories

### See how my account is doing right now

> As a pool user, I want the headline numbers from my account on the wall so I can confirm mining is paying out without
> logging into the pool.

- The Overview leads with the account's current hashrate — a 5-minute average from the pool, scaled to its own unit
  (TH/s, PH/s, EH/s), with the unit named in the label so the value and its unit never drift apart.
- *Todays Reward* shows today's estimated reward in BTC with an approximate fiat value beneath it.
- The workers panel counts the account's workers by state — active, low, and offline — each on its own colour-coded row.
  The larger frames lead with an all-workers total.
- Each size shows what its frame holds: the smallest is the hashrate alone, centered; the medium pairs hashrate and
  today's reward beside a compact workers panel; the large leads with the payout card over two stat cards and a
  sparkline; the full size runs stat tiles beside the chart and a roomy workers panel.
- Where the frame has room for it, the header carries the bound account's name, so a device showing two accounts is
  never ambiguous.

### Know when my next payout lands

> As a pool user, I want to see how close the next payout is and what the last one was, so I know the account is
> actually paying.

- The payout card shows *Next Payout in ~* with a live remaining time that ticks down on its own between refreshes, a
  progress meter for the payout period, and the last completed payout's amount in BTC.
- Only completed payouts count as the last payout; a pending or failed one is not shown as paid.

### Watch the hashrate trend

> As a pool user, I want a chart of recent hashrate so a dip is visible as a dip, not just a lower number.

- The Big Chart plots hashrate over the selected window against a left axis, with the active-worker count on a right
  axis, so a hashrate drop caused by workers dropping off is readable in one glance.
- Dashed gridlines mark the axis maximum, two thirds, one third, and zero. Hashrate ticks carry their own SI unit
  letter; worker ticks shorten to `k` form once the counts are large.
- The full-screen frame adds time labels along the bottom and marks each completed payout inside the window with an icon
  on the baseline — on-chain and Lightning payouts have distinct icons.
- Overview's larger frames carry the same history as a chart or a bare sparkline, without axes or labels.

### Choose how much history the chart covers

> As a pool user, I want to pick the chart's time span so I can look at the last few hours or the whole week.

- *Chart frame* selects 4 hours, 12 hours (the default), 24 hours, or 7 days.
- A 7-day window is thousands of 5-minute slots, more than the pool returns in one response; the widget follows the
  pool's pagination until the window is complete, then draws it as one series.

### Hide the worker breakdown

> As a pool user, I want to drop the worker stats when I only care about hashrate and payouts.

- *Worker states* hides the workers panel. With it off the widget also stops fetching worker data, and the chart's
  worker line goes with it — the display never polls for something it does not show.

### Bind my pool account once

> As a pool user, I want to enter my API key once, in the web app, and have the widget use it.

- The widget declares one required account slot, *Pool account*, of the Braiins Pool credential kind. The key is held by
  the device's saved accounts; the widget itself never sees it — the host substitutes it into the request as the pool's
  API key header.
- Until an account is bound, the widget shows bind instructions led by a QR code to the Deck web app, and names the
  network the device is on so the user knows where to browse from. The smallest frame shows the instruction alone.
- Re-binding or editing the account takes effect immediately; the widget refetches rather than waiting out its refresh
  interval.

### Tell "not loaded yet" apart from "nothing there"

> As a pool user, I want to know whether the widget is still fetching or the pool really has nothing to report, so I do
> not wait for a number that will never arrive.

- A slot that has not been answered yet shows a loading skeleton sized to the text it stands in for, so the layout does
  not jump when the value lands.
- A slot the pool answered with nothing shows an explicit dimmed callout instead — *No payouts yet* where the last
  payout would be, *No payout scheduled* in the meter's place. An empty meter track alone reads as a skeleton, which is
  why the meter slot carries words.
- A slot whose source failed before it ever answered says *Unavailable* instead of holding a skeleton that would never
  resolve. A source that had already delivered keeps its last good numbers, because one failed refresh is worth less
  than the data blanking it would cost.
- A zero that the pool actually reported is shown as a zero. It is a measurement, not an absence.

### Understand a key that cannot read my stats

> As a pool user, I want to be told when my API key is rejected, instead of watching the widget load forever.

- When the pool refuses a read (HTTP 401 or 403), both styles swap to a centered *Access denied* message that names the
  fix: the account's API key cannot read pool stats and must be reissued with monitoring access. Waiting does not help,
  so the widget says so rather than showing skeletons.
- Any successful read clears the state, so a corrected key recovers on the next refresh without touching the widget.

### Read numbers the way the rest of my device reads them

> As a user, I want the widget to follow the device's own formatting and timezone settings.

- Digit grouping and the decimal mark follow the device's localization setting.
- Chart time labels follow the device's 12/24-hour setting and timezone for windows up to 24 hours.

## Pool API

The widget reads Braiins Pool's FPPS API at `https://api.braiins.com/pool/v2`, authenticating with the bound account's
key. Every endpoint is polled once a minute with a 10-second timeout, and only when the current style, size, and worker
toggle actually display it.

| Endpoint                 | Feeds                                           | Polled for                                         |
| ------------------------ | ----------------------------------------------- | -------------------------------------------------- |
| `/user/hashrate/current` | the headline hashrate                           | every style and size                               |
| `/user/rewards/latest`   | today's reward in BTC and fiat                  | Overview, medium and larger                        |
| `/user/hashrate/history` | the hashrate chart and sparkline                | Big Chart; Overview large and full                 |
| `/user/workers/current`  | the workers-by-state panel and chart legend     | Big Chart medium and larger; Overview medium, full |
| `/user/workers/history`  | the chart's worker line                         | Big Chart medium and larger; Overview full         |
| `/user/financials`       | the next-payout estimate and meter              | Overview large and full                            |
| `/user/payouts/recent`   | the last payout, and the chart's payout markers | Overview large and full; Big Chart full            |

- The two worker endpoints are also gated by *Worker states*; with the toggle off, neither is polled.
- History and payout windows are requested as timestamp ranges and followed through the pool's cursor pagination until
  the window is complete.
- A failed or timed-out fetch is retried on the next pass, and the widget keeps the last good data meanwhile.

## Parameters

All parameters are manifest-driven widget settings, configurable from the web UI.

| Key             | Name          | Type    | Default    | Purpose                                                                          |
| --------------- | ------------- | ------- | ---------- | -------------------------------------------------------------------------------- |
| `style`         | Scene style   | string  | `overview` | `overview` for payout and worker stats, `big_chart` for the chart.               |
| `chart_frame`   | Chart frame   | string  | `hours_12` | Time span of the hashrate chart: `hours_4`, `hours_12`, `hours_24`, or `days_7`. |
| `worker_states` | Worker states | boolean | `true`     | Show the workers-by-state breakdown.                                             |

| Credential slot | Label        | Kind           | Required |
| --------------- | ------------ | -------------- | -------- |
| `pool`          | Pool account | `braiins-pool` | yes      |

## Constraints

- Rectangular viewports only, from 317×238 up to the Deck's full 1280×480. The layout is chosen from four size buckets
  rather than scaled continuously.
- The account slot is required: with nothing bound the widget polls nothing and shows the bind prompt.
- History is fetched in pages well under the pool's own page ceiling, so a wide window costs several requests; this
  keeps any single reply small enough for the widget to parse within its per-frame budget.
- Chart time labels for the 7-day window are UTC days rather than device-timezone days.
- The workers panel has rows for active, low, and offline workers; disabled workers are counted in the all-workers total
  but have no row of their own.
- Payout markers and chart time labels appear on the full-screen Big Chart only; smaller frames have no room for them.
- Solo mining to a Bitcoin address is out of scope: this widget reads a Braiins Pool account and needs one bound.
