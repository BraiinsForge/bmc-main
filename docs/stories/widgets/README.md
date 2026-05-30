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

### [Mining Info Widget](mining-info.md)

A widget that shows mining and Bitcoin network information across four selectable views — a miner overview, a miner
detail screen, a Bitcoin network screen, and a dense combined dashboard. Reads live miner stats over the BOS REST API
and Bitcoin price and network data from the Braiins public API, with responsive field degradation on smaller displays.
