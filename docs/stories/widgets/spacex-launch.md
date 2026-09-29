# SpaceX Launch Widget

The SpaceX Launch widget shows the next SpaceX launch — a live countdown plus mission details — across the four widget
sizes and in BMM101's own frame, with data from the Braiins Forge Nexus.

## User stories

### Count down to the next launch

> As a user, I want the Deck to count down to the next SpaceX launch so I know when it's happening.

- Shows the mission name and a live countdown to the scheduled launch time.
- The countdown ticks down every second between data refreshes.
- Once the launch time passes, the status reads `Launched`.
- While the first data is still loading it reads `Loading…`; BMM101 draws its frame with every value as `--`.

### Read the mission details

> As a user, I want the launch's key details so I understand what's flying.

- Shows the scheduled countdown, launch status (e.g. `Go for Launch`), rocket (e.g. `Falcon 9 Block 5`), and launch
  site.
- Shows the landing plan (e.g. `RTLS`, `ASDS`, `No attempt`), booster history (`Flight #1` or e.g. `3× flown`), payload
  type, and spacecraft when one is carried.
- The launch site and pad are abbreviated so they fit the panel (e.g. `CCSFS SLC-40`, `VSFB SLC-4E`, `Starbase OLP-2`).
- Payload types and Dragon names too long for BMM101's columns take a short form at every size, so each detail stays on
  one line: `Government/Top Secret` reads `Classified`, `Crew Dragon Endeavour` reads `Endeavour`, and
  `Cargo Dragon C208` reads `Dragon C208`. A payload type is matched by the word that sets it apart, so a reworded one
  still shortens.

### Use the space at each size

> As a user, I want the widget to use the available space well at every size.

- `full` shows a header, the mission name, both detail tables side by side, and an illustration of the rocket.
- `large` shows the header, mission name, and the detail tables stacked.
- `medium` shows a brand and mission header with the two tables side by side.
- `small` shows the mission name on one line, cut short with an ellipsis when it is long, and the core launch table.
- BMM101 has a 480x320 frame of its own: the header, the mission name large on one line or smaller over two when it is
  long, and all eight details in two ruled columns.
- The `Space X` brand never breaks across a line, however long the mission name beside it.
- The illustration matches the rocket (Falcon 9, Falcon Heavy, or a generic rocket for anything else).

### Stay accurate when data is unavailable

> As a user, I want the widget to stay accurate and not break when data is briefly unavailable.

- If a refresh fails, the last known launch stays on screen and the countdown keeps running.
- When there is no upcoming launch, it reads `No upcoming launches`.
- A connection or data error before any launch has loaded shows a short error message.
- On BMM101, both messages sit under the frame's header, in its type.

## Constraints

- Renders at the shared `small`, `medium`, `large`, and `full` sizes on rectangular viewports from 317x238 to 1280x480,
  and in the BMM101 panel's own frame; BMM100's 320x240 renders the `small` layout.
- Launch data comes from `https://nexus.braiinsforge.com/api/v1/data/spacex/next-launch`; the widget polls roughly every
  300 seconds and retries failures roughly every 30 seconds.
- Site and pad names, long payload types and Dragon names are abbreviated to fit the panels.
- The rocket illustration falls back to a generic rocket for non-Falcon vehicles.
