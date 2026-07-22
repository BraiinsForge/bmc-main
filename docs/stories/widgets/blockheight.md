# Block Height Widget

The block height widget displays the latest Bitcoin block height on the Deck, with an optional readout of when the block
was found. It fetches data from the Braiins Forge Nexus and refreshes about once a minute.

## User stories

### See the latest Bitcoin block height

> As a user, I want the Deck to show the current Bitcoin block height so I can keep an eye on the chain at a glance.

- The widget shows a "Block Height" header with a cube icon and the latest height as a large numeral.
- The height refreshes about once a minute from the Braiins Forge Nexus.
- While the height is still loading, or when no value has been received yet because fetches keep failing, it reads as
  `--`; failed fetches retry on their own without user action.

### Show or hide the block time

> As a user, I want to choose whether the widget shows the time of the latest block so I can keep the face minimal or
> informative.

- A *Show time and date* toggle controls whether the block's date and time appear under the height, below a *Found at*
  caption.
- The date and time follow the device's configured date format and timezone.
- When the block time is unknown or unavailable, it reads as `--` rather than as a wrong time.

### Choose the numeral weight

> As a user, I want to adjust the weight of the block height number so it reads well on my display.

- The block height numeral font weight is one of: regular, semi-bold, or bold.

## Constraints

- The widget renders at the shared `small`, `medium`, `large`, and `full` sizes and on the round BFM100 face; the
  numeral and the timestamp scale per size.
- Numeral weight and the timestamp toggle are manifest-driven widget parameters, configurable from the web UI.
- The date and time format follow the device's localization and timezone settings — they are not per-widget settings.
- Block data comes from the Braiins Forge Nexus; the widget polls roughly every 60 seconds and retries on failure.
