# SpaceX Launch Widget

The SpaceX Launch widget shows the next SpaceX launch — a live countdown plus mission details — across the four widget
sizes, with data from the Braiins Forge Nexus.

## User stories

### Count down to the next launch

> As a user, I want the Deck to count down to the next SpaceX launch so I know when it's happening.

- Shows the mission name and a live countdown to the scheduled launch time.
- The countdown ticks down every second between data refreshes.
- Once the launch time passes, the status reads `Launched`.
- While the first data is still loading it reads `Loading…`.

### Read the mission details

> As a user, I want the launch's key details so I understand what's flying.

- Shows the scheduled countdown, launch status (e.g. `Go for Launch`), rocket (e.g. `Falcon 9 Block 5`), and launch
  site.
- Shows the landing plan (e.g. `RTLS`, `ASDS`, `No attempt`), booster history (`Flight #1` or e.g. `3× flown`), payload
  type, and spacecraft when one is carried.
- The launch site and pad are abbreviated so they fit the panel (e.g. `CCSFS SLC-40`, `VSFB SLC-4E`).

### Use the space at each size

> As a user, I want the widget to use the available space well at every size.

- `full` shows a header, the mission name, both detail tables side by side, and an illustration of the rocket.
- `large` shows the header, mission name, and the detail tables stacked.
- `medium` shows a brand + mission header with the two tables side by side.
- `small` shows the mission name and the core launch table.
- The illustration matches the rocket (Falcon 9, Falcon Heavy, or a generic rocket for anything else).

### Stay accurate when data is unavailable

> As a user, I want the widget to stay accurate and not break when data is briefly unavailable.

- If a refresh fails, the last known launch stays on screen and the countdown keeps running.
- When there is no upcoming launch, it reads `No upcoming launches`.
- A connection or data error before any launch has loaded shows a short error message.

## Constraints

- Renders at the shared `small`, `medium`, `large`, and `full` sizes on rectangular viewports from 317x238 to 1280x480.
- Launch data comes from `https://nexus.braiinsforge.com/api/v1/data/spacex/next-launch`; the widget polls roughly every
  300 seconds and retries failures roughly every 30 seconds.
- Site and pad names are abbreviated to fit the panels; long upstream names are shortened.
- The rocket illustration falls back to a generic rocket for non-Falcon vehicles.
