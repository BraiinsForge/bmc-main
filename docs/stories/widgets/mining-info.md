# Mining Info Widget

The mining info widget shows mining and Bitcoin network information on the Deck. A single *view* parameter selects one
of four screens — a miner overview (`mining`), a miner detail screen (`geek`), a Bitcoin network screen (`network`), and
a dense combined dashboard (`info_overload`). Miner-local data comes from a BOS miner over its REST API; Bitcoin price
and network data come from the Braiins public API. The two sources are independent, so one can fail without blanking the
other.

## User stories

### Choose what the widget shows

> As a user, I want to pick which screen the widget shows so the same widget can act as a miner overview, a detail
> screen, a network screen, or a full dashboard.

- The *View* parameter is one of: *Mining*, *Geek*, *Network*, or *Info Overload*; the default is *Mining*.
- *Mining* and *Geek* are miner overview screens; *Network* is a Bitcoin network screen; *Info Overload* is a combined
  dashboard.
- Changing the view takes effect without removing the widget or losing its other settings.

### See my miner at a glance (Mining)

> As a user, I want a quick overview of my miner so I can confirm it is running healthily.

- The *Mining* view shows current hashrate (TH/s), temperature (°C), power consumption (W), MCR (%), fan speed (%), and
  the miner's IP address.
- Temperature is shown in Celsius as a board-to-chip range (e.g. *61-74*), matching the BOSer miner screen.
- Rows fill the height of the display rather than packing at the top.

### See miner detail (Geek)

> As a user, I want a detail screen that pairs miner stats with the Bitcoin price so I can read deeper status in one
> place.

- The *Geek* view shows current hashrate, temperature, power consumption, miner uptime, the miner's IP address, and the
  current BTC price.
- Uptime reads compactly as days, hours, and minutes (e.g. *2d 3h 57m*).

### Watch the Bitcoin network (Network)

> As a user, I want a Bitcoin network screen so I can follow difficulty, fees, hashprice, and block height without my
> miner.

- The *Network* view shows network hashrate (EH/s), the previous difficulty adjustment (%), fees over the last 144
  blocks (BTC), block height, hashprice (per TH/day), and the BTC price.
- On larger displays it also shows the estimated next difficulty adjustment (%), epoch progress (%), and the fee figure
  gains the fee share as extra info (e.g. *0.012 BTC | 1.4 %*).
- This screen uses only public Bitcoin data, so it works even when no miner is reachable.

### See everything at once (Info Overload)

> As a user, I want a dense dashboard that combines my miner and the Bitcoin network so I can monitor both from one
> screen.

- The *Info Overload* view leads with a Bitcoin band: the 24-hour price change and the current BTC price. The change
  reads green when up and red when down.
- Below the band it shows a field grid: hashrate, power consumption, block height, and miner uptime.
- On larger displays the grid expands to add the estimated and previous difficulty adjustments, epoch progress, the fee
  share over 144 blocks, and hashvalue (SAT/TH/day).

### See my miner on a round display

> As a user with a round-display Deck, I want the miner overview to fill the circular screen and show miner health at a
> glance.

- On the round 480×480 display (BFM100), all four screens use dedicated circular layouts. While the Mining and Geek have
  round gauges, the other two do not have specific round elements, they are just made to fit on a round screen.
- *Mining* and *Geek* center the current hashrate inside a 28-segment ring and place four stats in the quadrants around
  it. *Mining*'s quadrants are power consumption, MCR, temperature, and fan speed; *Geek* swaps MCR for efficiency
  (J/TH) and fan speed for the BTC price. In these compact clusters temperature reads as the chip temperature alone, not
  the board-to-chip range shown on the rectangular screens.
- The ring's color and fill reflect the miner's state derived from its MCR: green when running well, amber when
  underclocked, purple when overclocked, and red with a single lit tick when the miner is not hashing. While the miner
  is hashing but its MCR is unavailable, the ring stays unlit and the hashrate label reads neutral rather than implying
  a state.
- *Info Overload* arranges its fields into five horizontal bands with the Bitcoin price band across the middle; it drops
  only the network hashrate and hashprice relative to the large rectangular layout.

### Point the widget at my miner

> As a user, I want to tell the widget where my miner is and how to log in so it can read live stats.

- The *Miner URL* parameter is the base BOS REST API URL of the miner; it defaults to `http://localhost/api/v1`.
- The *Miner password* parameter is the password for the miner's `root` login; it defaults to `root`. The login username
  is always `root`.
- The widget logs in, caches the session token, and re-authenticates on its own if the token expires; miner stats
  refresh roughly every five seconds.
- When the password is empty or the login fails, the miner fields stay unavailable while any public Bitcoin fields keep
  rendering.

### Choose the currency

> As a user, I want to choose the currency for Bitcoin values so prices read in the unit I think in.

- The *Currency* parameter is one of *USD* or *EUR*; the default is *USD*.
- It applies to the BTC price and to the hashprice and hashvalue figures.

### Trust what the numbers say

> As a user, I want clear placeholders when data is missing so I never mistake a stale or absent value for a real one.

- Unavailable values read as `N/A`, whether they are miner-local or public Bitcoin figures.
- Failed fetches retry on their own without user action, and the widget keeps the last good data rather than blanking
  the screen. When a refresh keeps failing — miner or public Bitcoin — the last good values stay on screen and a
  `Stale data` banner appears until the next successful fetch.
- Numbers use the device's configured number format for digit grouping and the decimal mark.

## Constraints

- The widget renders on rectangular viewports from 320×240 up to 1280×480 and on the round 480×480 viewport. The primary
  rectangular targets are BMM100 (320×240) and BMM101 (480×320); BMC100 rectangular viewports are best-effort. The round
  viewport targets the BFM100.
- On rectangular viewports, layout adapts in two bands: a small band (320 wide or 240 tall and below) uses a two-column
  grid, tighter spacing, and hides secondary fields; a larger band uses a three-column grid, wider spacing, and shows
  the full field set. Font sizes are fixed across viewports — fields are hidden rather than shrunk.
- The round viewport uses separate circular layouts for all four screens. *Network* carries no round-specific visuals;
  it re-stacks its eight public stats into chord-fitted rows (1-2-2-2-1) centered on the circle rather than reusing the
  rectangular layout.
- *View*, *Miner URL*, *Miner password*, and *Currency* are manifest-driven widget parameters, configurable from the web
  UI.
- Number formatting follows the device's localization system setting; it is not a per-widget setting.
- The *Miner password* is stored and shown as ordinary widget text because the manifest system has no secret-parameter
  type yet. This is a known limitation.
- Field sets, labels, and units mirror the BOSer BMM screens.
- Miner-local data (hashrate, temperature, power, MCR, fan, uptime, IP) comes from the miner's BOS REST API and
  refreshes about every five seconds; Bitcoin price and network data come from `public-api.braiins.com` and refresh
  about every sixty seconds. The two sources are independent and retry on failure.
- The small BTC price graph from the BOSer *info_overload* screen is not included.
