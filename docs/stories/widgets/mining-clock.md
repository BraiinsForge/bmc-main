# Mining Clock Widget

The mining clock widget is the clock widget reshaped for a miner. It shows the current time on a round analog dial and
wraps that dial in two live gauge rings — an outer ring for the miner's hashrate and an inner ring for its power
consumption. It keeps the clock's date window, timezone label, numeral weight, seconds hand, and next-alarm indicator,
and reads live stats from a BOS miner over its REST API. It runs full-screen on the round BFM100 panel.

## User stories

### Read the time on the round dial

> As a user, I want a normal analog clock at the centre of the widget so it still works as a clock first.

- The face is always the round analog dial — rotating hour, minute, and (optionally) second hands over a dial, with the
  day-of-month in a window inside the dial.
- The dial fills the shorter side of the viewport and downscales to fit rather than overflowing, so it stays whole on
  the round panel.
- Unlike the standalone clock widget, the face style is fixed: there is no analog-rectangular or digital option, because
  the gauge rings are drawn around a round dial.

### See live hashrate at a glance

> As a user, I want the widget to show how hard my miner is working without leaving the clock face.

- The outer ring is the hashrate gauge. It fills clockwise against the miner's configured tuner targets and carries a
  curved `TH/s` label that rides the end of the lit arc.
- The sweep is anchored to three configured points: the minimum target sits a quarter of the way around the ring, the
  default target three-quarters, and the maximum at the full ring, with the fill interpolating linearly between them. A
  miner running at its default target therefore fills about three-quarters of the ring.
- The ring's colour reflects the miner's state — see [Read the miner's state](#read-the-miners-state).
- The targets come from the miner's tuner constraints. When they are unavailable the ring reads empty even if a current
  hashrate is known — there is no scale to fill against — but the `TH/s` label still shows the live value.

### See power consumption at a glance

> As a user, I want to see how much power my miner is drawing alongside the hashrate.

- The inner ring is the power gauge. It fills clockwise with the miner's approximated power consumption and carries a
  curved `W` label at the end of the lit arc.
- Like the hashrate ring, its sweep is anchored to the configured power targets: the minimum at a quarter of the ring,
  the default at three-quarters, and the maximum at the full ring, interpolating linearly between.
- It shares the hashrate ring's single colour rather than carrying a load ramp of its own, so the two rings always read
  as one instrument. When the power targets are unavailable the inner ring reads empty while the outer ring still fills.

### Read the miner's state

> As a user, I want the gauge colour to tell me whether my miner is running where I expect, so I can spot over- or
> under-performance at a glance.

- Both rings are coloured by one state, derived from the miner's hashrate relative to its **default** hashrate target:
  - **Normal** (green) — the hashrate is within a small tolerance of the default target (currently ±5%); the miner is
    running where it was configured.
  - **Overclocked** (purple) — the hashrate is at least the tolerance above the default target.
  - **Underclocked** (amber) — the hashrate is at least the tolerance below the default target.
  - **Not hashing** (red, a single lit tick) — the miner reports essentially no hashrate.
  - **No state** (gray, unlit) — the hashrate or its target is unavailable, so no state can be derived; the labels read
    neutral rather than implying one.
- The normal and underclocked states render as a gradient that deepens from dark to bright along the sweep; overclocked
  is solid purple and not-hashing is solid red.
- The power ring is positioned by the power target but never carries its own colour — it always takes this same state
  colour, even though its fill length is driven by power rather than hashrate.

### Watch the rings animate in

> As a user, I want the gauges to come to life when the widget appears so it reads as a live instrument, not a static
> picture.

- On the first frame both rings start empty and then sweep out to their real values, even when miner data is already
  available when the widget loads.
- Whenever a reading changes, the ring animates its fill from the old value to the new one rather than jumping.

### Show or hide the date

> As a user, I want to choose whether the clock shows the date so I can keep the dial minimal or informative.

- A *Show date* toggle controls the date window — a small ringed circle inside the dial holding the day of the month.
- The date window appears at the larger render sizes; smaller sizes omit it to keep the dial legible.

### Show or hide seconds

> As a user, I want to choose whether seconds are shown so the clock can tick precisely or stay calm.

- A *Show seconds* toggle shows or hides the second hand.

### Show, hide, or override the timezone

> As a user, I want the clock to show a timezone other than the device's so I can track time in another location.

- A *Show timezone* toggle controls whether a timezone label is shown inside the dial.
- The label stacks the city name over its signed `±HH:MM` UTC offset.
- By default the clock follows the device's system timezone; an optional timezone override makes it display a chosen
  IANA timezone instead.
- An unresolvable timezone is shown as unknown rather than as a wrong time.

### Choose the numeral weight

> As a user, I want to adjust the weight of the clock's numbers so they read well on my display.

- The numeral and digit font weight is one of: regular, semi-bold, or bold. The default is semi-bold.

### Point the widget at my miner

> As a user, I want to tell the widget where my miner is and how to log in so it can read live stats.

- The *Miner URL* parameter is the base BOS REST API URL of the miner; it defaults to `http://localhost/api/v1`.
- The *Miner password* parameter is the password for the miner's `root` login; it defaults to `root`. The login username
  is always `root`.
- The widget logs in, caches the session token, and re-authenticates on its own when the token expires; hashrate and
  power refresh roughly every five seconds, while the tuner constraints that scale the rings are fetched once per login
  (they change only when the miner is re-tuned) and re-fetched when the widget re-authenticates.
- When the password is empty or the login fails, the gauges read empty and the rings keep their stable transition slots.

### Trust what the numbers say

> As a user, I want the gauges to read empty when data is missing so I never mistake an absent value for a real one.

- A reading that has not loaded shows `N/A` in its ring label and a gray ring.
- Stale readings are kept, but "Stale data" overlay appears.
- Failed fetches retry on their own without user action.
- Numbers use the device's configured number format for digit grouping and the decimal mark.

## Constraints

- The widget renders only on the round 480×480 viewport; it is a round-display widget by design.
- The date window and next-alarm indicator are gated by render size: the date window appears at the larger sizes, the
  alarm row only at the full size.
- *Miner URL*, *Miner password*, *Numbers font style*, *Show date*, *Show seconds*, *Show timezone*, and the timezone
  override are manifest-driven widget parameters, configurable from the web UI.
- Both rings scale against the miner's tuner constraints — the configured min / default / max for hashrate and for power
  — with the minimum at a quarter of the ring, the default at three-quarters, and the maximum at the full ring. When a
  target is unavailable, its ring reads empty rather than guessing a scale.
- Day / night appearance follows the device-wide night mode signal — the clock recolours itself; it is not a per-widget
  setting. See [Night Mode](../night-mode.md).
- The next alarm and the 12-/24-hour time format come from device system state, not from widget configuration.
- Number formatting follows the device's localization system setting; it is not a per-widget setting.
- The *Miner password* is stored and shown as ordinary widget text because the manifest system has no secret-parameter
  type yet. This is a known limitation, shared with the [Miner Info widgets](miner-info-mining.md).
- Miner-local data comes from the miner's BOS REST API; the widget reads `/miner/stats` for live hashrate and power and
  `/configuration/constraints` for the gauge target scales, authenticating via `/auth/login`.

## Platforms

The mining clock is a round-display widget. Unlike the standalone clock, it does not adapt to rectangular panels — the
two gauge rings only close cleanly around a round dial.

| Platform | Display   | Shape | Renders as                                 |
| -------- | --------- | ----- | ------------------------------------------ |
| BFM100   | `480x480` | round | the round analog dial with two gauge rings |
