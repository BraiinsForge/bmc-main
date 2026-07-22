# Halving Countdown Widget

The halving countdown widget counts down to the next Bitcoin halving on the Deck, and — on the larger sizes — shows when
the halving is predicted to happen and how many blocks are left. It reads a server-computed prediction from the Braiins
Forge Nexus and refreshes about once a minute.

## User stories

### Count down to the next halving

> As a user, I want the Deck to count down to the next Bitcoin halving so I know how far away it is at a glance.

- The widget shows a "Halving Countdown" header and the time remaining as days, hours, and minutes.
- The countdown ticks down on its own and stays current between refreshes.
- While the prediction is still loading, or when it cannot be fetched, the numerals read as `-`.

### See the predicted date and blocks remaining

> As a user, I want to see when the halving is predicted and how many blocks are left so I get the fuller picture, not
> just a countdown.

- On the larger sizes, two tiles appear below the countdown: the predicted halving date and time, and the number of
  blocks remaining with the target block height.
- The predicted date and time follow the device's date format, time format, and timezone, and show the timezone
  alongside the time (for example, `Prague (+2)`).
- The smaller sizes and the round face show the countdown on its own, without the two tiles.

### Choose the numeral weight

> As a user, I want to adjust the weight of the countdown numerals so they read well on my display.

- The countdown numeral font weight is one of: regular, semi-bold, or bold.

## Constraints

- The widget renders at the shared `small`, `medium`, `large`, and `full` sizes and on the round BFM100 face; the layout
  and type scale per size.
- The prediction comes from the Braiins Forge Nexus; the widget polls roughly every 60 seconds and keeps counting down
  from the last good prediction between polls.
- Numeral weight is a manifest-driven widget parameter, configurable from the web UI.
- The predicted date, time, and timezone follow the device's localization and timezone settings — they are not
  per-widget settings.
