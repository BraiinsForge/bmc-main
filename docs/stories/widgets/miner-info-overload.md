# Miner Info — Info Overload Widget

A dense dashboard combining one miner's readings with Bitcoin network figures. Miner data comes from a BOS miner over
its REST API; the network figures come from the Braiins public API. The two sources are independent, so one can fail
without blanking the other.

Add the widget once per miner to watch several at a time.

## User stories

### See everything at once

> As a user, I want a dense dashboard that combines my miner and the Bitcoin network so I can monitor both from one
> screen.

- The widget leads with a Bitcoin band: the 24-hour price change and the current BTC price. The change reads green when
  up and red when down.
- Below the band a field grid shows hashrate, power consumption, block height, the estimated and previous difficulty
  adjustments, epoch progress, miner uptime, the fee share over the last 144 blocks, and hashvalue (SAT/TH/day).
- The band carries a small sparkline of the last day's price between the change and the price itself.

### See it on the Mini Miner

> As a user with a BMM101, I want the dashboard laid out for its own 480×320 screen rather than borrowed from another.

- The screen opens with the widget's icon and name, then the Bitcoin band on the black background, then a hairline, then
  the nine figures in three rows spread over the rest of the height.
- The band spreads the 24-hour change, the sparkline and the price across the width; the currency symbol sits smaller
  beside the price.
- Each figure's unit is set smaller than its value, and the labels smaller still.

### Read it on a small screen

> As a user with a small display, I want the dashboard to stay readable rather than run off the edge.

- On screens narrower than the 480-wide design the grid drops to two columns and shows a reduced field set: hashrate,
  power consumption, miner uptime and block height.
- Block height moves into the second row rather than being dropped, so the narrow grid still carries a network figure
  alongside the miner ones.
- The price sparkline is omitted at that size; the band keeps the change and the price.

### See it on a round display

> As a user with a round-display Deck, I want the dashboard to fit the circular screen.

- On the round 480×480 display (BFM100) the fields arrange into five horizontal bands with the Bitcoin price band across
  the middle.
- This screen has no gauge ring — it is the one Miner Info widget that reads no tuner targets.

### Keep reading whichever source still answers

> As a user, I want a failure on one side to leave the other side readable so a dead miner does not cost me the network
> figures, and no internet does not cost me my miner.

- Miner fields and network fields are fetched independently. When the miner is unreachable its hashrate, power and
  uptime read `N/A` while block height, the difficulty adjustments, epoch progress, fees and hashvalue keep updating.
- A failure banner names which source failed: `Cannot authenticate` for a miner that turns the credentials away — a
  refused login in remote mode, a rejected token in local mode, where there is no login to refuse —
  `Failed to load: Miner` for one that never answers or fails its telemetry, and `Failed to load: Network` for the
  public API.
- The banner floats over the screen rather than replacing it, so the half that still works stays readable underneath.

### Point the widget at my miner

> As a user, I want the widget to read my own miner without a password, and to be able to watch another miner on my
> network by giving its address and login.

- The widget has two account slots and uses exactly one of them:
  - *Local BOS token*: the miner this display belongs to. Bind the *Local BOS API* account (present from first boot on
    BFM100 and BMM) and keep the *Miner URL* at `http://localhost/api/v1`. No password is entered, and the widget keeps
    working after the miner password changes.
  - *Remote miner login*: a miner elsewhere on the network. Set the *Miner URL* to its API base, for example
    `http://10.0.0.5/api/v1`, and bind an account holding that miner's password; the username is always `root`. The
    widget logs in, caches the session token, and re-authenticates on its own when it expires.
- With neither slot bound the widget shows "Bind a BOS account"; with both it shows "Bind one BOS account, not both".
  The local token is never sent to the *Miner URL*.
- A remote username or password containing `"` or `\` is not supported.
- Miner stats refresh roughly every five seconds and the network figures about every sixty. Pointing the widget at a
  different miner clears the readings first, so one miner's figures are never shown under another's address.

### Trust what the numbers say

> As a user, I want clear placeholders when data is missing so I never mistake a stale or absent value for a real one.

- Unavailable values read as `N/A`, whether they are miner-local or public Bitcoin figures.
- A price history too short to have a shape leaves the sparkline's column empty rather than drawing a flat line that
  would imply no change.
- Failed fetches retry on their own without user action. When a refresh keeps failing the last good values stay on
  screen under a `Stale data` banner until the next successful fetch.
- Numbers use the device's configured number format for digit grouping and the decimal mark.

## Constraints

- The widget renders on rectangular viewports from 317×238 up to 480×320 and on the round 480×480 viewport. The
  rectangular targets are the BMM100 (320×240), the BMM101 (480×320), and the BMC100's 1×1 slot (317×238); the round
  viewport targets the BFM100. The BMM100 and the BMC100 slot share one layout; the BMM101 has its own.
- The wider BMC100 views are deliberately unsupported: the grids are drawn at fixed block widths for their screens, so
  on a larger one a grid would sit in a corner rather than fill it. A design for those sizes does not exist yet.
- Font sizes are fixed across viewports — fields are hidden rather than shrunk.
- *Miner URL* is a manifest-driven widget parameter; the two account slots are credential bindings, chosen from saved
  accounts in the web UI. No password is stored in widget parameters.
- Prices read in US dollars. There is no currency parameter.
- Number formatting follows the device's localization system setting; it is not a per-widget setting.
- Miner data comes from the miner's BOS REST API and refreshes about every five seconds; the network figures come from
  `public-api.braiins.com` and refresh about every sixty. The two retry independently.
