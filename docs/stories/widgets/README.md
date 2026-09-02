# BMC Widget Stories

Documentation of official widgets for the Braiins Deck. Each document captures user stories, behavior, and constraints
for a single widget.

## Widgets

### [Bitcoin Mining Data Widget](bitcoin-mining-data.md)

A dashboard for current Bitcoin difficulty, adjustment timing, hashprice, BTC-USD price, mining economics, block
production, and network hashrate. It expands from a compact mining overview into historical charts and a full network
dashboard across the four rectangular widget sizes, follows the Braiins Forge Nexus refresh lifetime, and distinguishes
loading, stale, unavailable, and rate-limited data.

### [Block Height Widget](blockheight.md)

A widget that displays the latest Bitcoin block height with an optional *found at* date and time, a configurable numeral
weight, and an automatic refresh against the Braiins Forge Nexus. Renders at all four widget sizes and on the round
BFM100 face.

### [Braiins Pool Widget](braiins-pool.md)

Live hashrate, worker, and payout stats for one Braiins Pool account, bound as a saved account so the API key is entered
once. Renders as an Overview of payout and worker stats or as a Big Chart of hashrate history over a selectable window,
with the active-worker count on a second axis and completed payouts marked on the full-screen chart. Distinguishes
not-yet-loaded from genuinely empty, and names the fix when the API key cannot read pool stats.

### [Clock Widget](clock.md)

A clock widget with an analog (round or rectangular) or digital face, optional date, seconds, and timezone readouts, a
configurable numeral weight, and a next-alarm indicator. Renders at all four widget sizes on BMC100, scales to fit the
BMM100, BMM101, and BFM100 panels, and recolours for night mode.

### [Fleet Management Widget](fleet-management.md)

An at-a-glance view of every Bitcoin miner on the local network. Discovers BOS, Braiins OS Libre, and AxeOS (Bitaxe /
NerdQAxe++) miners over mDNS, polls each for live telemetry, and rolls them up into a fleet total, a per-model
breakdown, and a per-device detail view — with hashrate trend charts, OK/degraded/off health against each miner's
nominal, per-family credentials, and a selectable chart time range.

### [Formula 1 Widget](formula-1.md)

A widget that follows a Grand Prix season — the drivers' championship standings, the next race weekend with its schedule
and circuit, one driver's career card, and live timing for a race, qualifying or practice session in progress. Left on
Automatic it picks the view for itself: the live board while a session runs, otherwise the next race, otherwise the
standings. Reads the Braiins Forge Nexus, follows device localization and timezone, and can show session times on the
circuit's clock or the deck's. Renders at all four widget sizes on rectangular viewports.

### [Halving Countdown Widget](halving-countdown.md)

A widget that counts down to the next Bitcoin halving — days, hours, and minutes remaining — and, on the larger sizes,
shows the predicted halving date and the blocks remaining with the target block height. Reads a server-computed
prediction from the Braiins Forge Nexus, follows device localization and timezone, and offers a configurable numeral
weight. Renders at all four widget sizes and on the round BFM100 face.

### [ISS Position Widget](iss-position.md)

A live tracker for the International Space Station — at full size a 3D globe with the station marker, its orbital ground
track, and a day/night terminator; at smaller sizes a position and telemetry panel (ground position, altitude, velocity,
sunlit/eclipsed). Pulls data from the Braiins nexus service and propagates the live position on-device between
refreshes. Rectangular viewports only.

### [Miner Info — Mining Widget](miner-info-mining.md)

A miner overview: hashrate, temperature, power, MCR, fan speed and IP address, read over the BOS REST API. On the round
panel the hashrate sits inside a gauge ring coloured by how the miner tracks its tuner target. Needs no internet.

### [Miner Info — Geek Widget](miner-info-geek.md)

A miner detail screen pairing hashrate, temperature, power, uptime and IP address with the current BTC price. Miner and
price are fetched independently, so either can fail without blanking the other. Carries the same round gauge as Mining.

### [Miner Info — Info Overload Widget](miner-info-overload.md)

A dense dashboard combining one miner with the Bitcoin network — price and 24-hour change over a grid of difficulty
adjustments, epoch progress, block height, fees and hashvalue. Drops to two columns on the smallest displays.

### [Mining Clock Widget](mining-clock.md)

The clock widget reshaped for a miner: a round analog dial wrapped in two live gauge rings — an outer hashrate ring and
an inner power ring — that reads stats from a BOS miner over its REST API. Keeps the clock's date, timezone, seconds,
numeral weight, and next-alarm features. Renders full-screen on the round BFM100 panel only.

### [SpaceX Launch Widget](spacex-launch.md)

A widget that counts down to the next SpaceX launch and shows its mission details — status, rocket, launch site,
landing, booster reuse, payload, and spacecraft — across the four widget sizes, with a rocket illustration at the full
size. Reads launch data from the Braiins Forge Nexus and keeps the last known launch on screen when a refresh fails.

### [Nameday Widget](nameday.md)

A widget that shows whose nameday it is today for a chosen country. Displays a country header with a flag, today's names
as a large headline, and an optional date readout. Reads the day's names from the public `nameday.abalin.net` API,
follows the device timezone to pick the correct local day, refreshes at local midnight, and truncates long name lists
with an ellipsis. Renders at all four widget sizes on rectangular viewports.

### [Picture of the Day Widget](picture-of-the-day.md)

A widget that shows the current NASA Astronomy Picture of the Day, captioned with its title and the photographer's
credit. Follows the feed rather than the clock: a half-hourly metadata check names the published date, and a picture is
downloaded only when that date changes, so an unsynchronised device clock cannot make it ask for the wrong day. Shows
the picture whole, with an optional title; the credit, where the feed names one, cannot be turned off. Renders at all
four widget sizes on rectangular viewports.

### [Random Facts Widget](random-facts.md)

A widget that shows a single random factoid, large and centered on the tile, under a fixed "Random Facts" header. Pulls
a fresh fact from the public `api.viewbits.com` useless-facts API every few minutes, auto-fits the text to the available
area, and keeps the last shown fact on screen when a refresh fails. Has no parameters and renders at all four widget
sizes on rectangular viewports.

### [Weather Widget](weather.md)

A widget that shows current weather and forecast data for a chosen location, with hourly and daily forecast layouts
across the shared rectangular widget sizes. Reads weather data from the Braiins Forge Nexus API, follows device
localization and timezone settings, and lets users choose whether forecast times use the location or device timezone.

### [Ticker List Widget](ticker-list.md)

A widget that lists up to eight financial instruments — stocks, indices, currency pairs, or cryptocurrency pairs — one
per row, each with its name, a sparkline over a selectable time period, the current price, and a signed change badge.
Reads prices and instrument names from the Braiins Forge Nexus, fetches every row independently so one bad symbol only
degrades its own row, and adapts from eight rows in two columns at full size down to two chartless rows at small.

### [Single Ticker Widget](ticker-single.md)

A widget that follows one financial instrument in depth, either as a sparkline behind a large price readout or as a
candlestick chart with a price axis, a volume strip, and time labels. Reads price history from the Braiins Forge Nexus
over a selectable time period, dims when the instrument's market is closed, and keeps the last known chart on screen
when a refresh fails.
