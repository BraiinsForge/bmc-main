# BMC Widget Stories

Documentation of official widgets for the Braiins Deck. Each document captures user stories, behavior, and constraints
for a single widget.

## Widgets

### [Block Height Widget](blockheight.md)

A widget that displays the latest Bitcoin block height with an optional block date and time, a configurable numeral
weight, and an automatic refresh against the Braiins public API. Renders at all four widget sizes.

### [Clock Widget](clock.md)

A clock widget with an analog (round or rectangular) or digital face, optional date, seconds, and timezone readouts, a
configurable numeral weight, and a next-alarm indicator. Renders at all four widget sizes on BMC100, scales to fit the
BMM100, BMM101, and BFM100 panels, and recolours for night mode.

### [Fleet Management Widget](fleet-management.md)

An at-a-glance view of every Bitcoin miner on the local network. Discovers BOS, uBOS, and AxeOS (Bitaxe / NerdQAxe++)
miners over mDNS, polls each for live telemetry, and rolls them up into a fleet total plus a per-model breakdown, with
per-family credentials, manual-host fallback, and model/family filtering. Falls back to a summary-only screen on smaller
viewports.

### [ISS Position Widget](iss-position.md)

A live tracker for the International Space Station — at full size a 3D globe with the station marker, its orbital ground
track, and a day/night terminator; at smaller sizes a position and telemetry panel (ground position, altitude, velocity,
sunlit/eclipsed). Pulls data from the Braiins nexus service and propagates the live position on-device between
refreshes. Rectangular viewports only.

### [Mining Clock Widget](mining-clock.md)

The clock widget reshaped for a miner: a round analog dial wrapped in two live gauge rings — an outer hashrate ring and
an inner power ring — that reads stats from a BOS miner over its REST API. Keeps the clock's date, timezone, seconds,
numeral weight, and next-alarm features. Renders full-screen on the round BFM100 panel only.

### [Mining Info Widget](mining-info.md)

A widget that shows mining and Bitcoin network information across four selectable views — a miner overview, a miner
detail screen, a Bitcoin network screen, and a dense combined dashboard. Reads live miner stats over the BOS REST API
and Bitcoin price and network data from the Braiins public API, with responsive field degradation on smaller displays.

### [Weather Widget](weather.md)

A widget that shows current weather and forecast data for a chosen location, with hourly and daily forecast layouts
across the shared rectangular widget sizes. Reads weather data from the Braiins Forge Nexus API, follows device
localization and timezone settings, and lets users choose whether forecast times use the location or device timezone.
