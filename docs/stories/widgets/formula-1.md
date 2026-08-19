# Formula 1 Widget

The Formula 1 widget follows a Grand Prix season: the drivers' championship standings, the upcoming race weekend and its
schedule, one driver's career card, and live timing while a session is running. It reads the Braiins Forge Nexus, which
also hosts the artwork the screens draw. Left on **Automatic** the widget picks the view for itself, so the same tile
shows the live board on a race Sunday and the standings on a quiet week. All four widget sizes are supported.

## User stories

### Watch a session as it runs

> As a fan, I want the timing screen on the wall during a session so I can follow the order without a second screen.

- Whichever session is running — race, qualifying, or practice — is drawn as a timing board, and each has the columns
  that suit it: the race shows gap, interval, last lap and tyre; qualifying shows the lap that ranks each driver;
  practice shows the best lap, its compound, and the gap to the session's quickest.
- Sector times carry their own colour, so a purple sector reads as the session's quickest at a glance and a green one as
  the driver's own best.
- A car in the pits reads as itself rather than as a slow lap.
- The full-screen qualifying board splits the whole field into two tables side by side, so every driver has a row —
  including whoever sorted to the tail. The other boards seat the leading ten, or five on the narrower frames.

### Know when the next race is

> As a fan, I want the next Grand Prix and its session times so I know when to be in front of the TV.

- The Next Race screen names the weekend — the Grand Prix and its country — and carries the circuit's own figures on
  every frame: the lap count, the circuit length, the race distance, the DRS zones, and the tyre compounds it is run on.
- Beside them stands the schedule, every session with its start time, under the weekend's date range. The smallest frame
  has room for neither: it keeps the circuit's figures alone, and names the country rather than the Grand Prix.
- A sprint weekend shows the sprint and its own qualifying in place of two practices, so the schedule matches the
  weekend actually being run.
- The full-screen frame adds the circuit map, drawn beside the two columns.
- With no race announced — between seasons, or before the first fetch lands — the screen drops its whole body and reads
  *Next race unavailable*, rather than a card with every value missing.

### Read the schedule in my own time, or the circuit's

> As a fan abroad, I want to know what time a session starts *for me*, not at the track.

- By default the schedule stays on the circuit's clock, the way a race weekend is always quoted.
- *Use my local time* switches the session times to the deck's own timezone, and the schedule updates as soon as the
  setting changes rather than at the next refresh. The weekend's date range does not move either way.

### Follow the championship

> As a fan, I want the drivers' championship table so I can see who is actually winning the season.

- The standings list drivers in championship order with their points, their constructor and its mark, and — where the
  frame has room — their nationality flag and code. Ten drivers fit the wider frames, five the narrower ones.
- Between seasons the table has no rows, and the widget reads *Standings unavailable* rather than showing last year's
  order. It reads the same while the first fetch is still in flight, so an off-season and a slow start look alike.

### Follow one driver

> As a fan of one driver, I want their card on the wall rather than a table I have to find them in.

- Driver Statistics shows the chosen driver's portrait, name and number, their constructor and its mark, championship
  position and points, and their career figures: Grand Prix wins, world titles, F1 debut season.
- The wider frames add the personal details — age, weight, height, nationality with its flag, and the race engineer.
- A driver whose card the deployment has not filled in shows what is known rather than blanking the screen.

### Let the widget choose the screen

> As a fan, I want one tile that is useful all season without me changing a setting every weekend.

- **Automatic** shows whichever session is live, preferring the race; failing that the next race; failing that the
  standings, which every season has, so the chain always ends somewhere.
- Choosing a view explicitly always wins, and that view is shown — empty at first — while its data loads.

### Keep working when the season data is not there yet

> As a fan, I want the widget to degrade honestly rather than show me a blank tile.

- A fresh Nexus deployment derives its career data before it can answer, and the screens that need it show as
  unavailable meanwhile, then fill in on their own once the deployment is up — no restart, no settings change.
- A failed refresh keeps the last good data on screen rather than emptying it, and an image that has not arrived yet
  leaves its placeholder.

## Nexus API

The widget reads the Braiins Forge Nexus at `/api/v1/data/formula-1/…`. Each resource is polled only while the chosen
view reads it.

| Resource        | Feeds                                             | Polled for           |
| --------------- | ------------------------------------------------- | -------------------- |
| `standings`     | the championship table                            | Standings, Automatic |
| `next-race`     | the weekend, its schedule and circuit             | Next Race, Automatic |
| `live-race`     | the race timing board                             | Automatic            |
| `live-quali`    | the qualifying board                              | Automatic            |
| `live-practice` | the practice board                                | Automatic            |
| `driver/<slug>` | the chosen driver's card                          | Driver Statistics    |
| `driver-stats`  | that driver's season figures, joined by driver id | Driver Statistics    |
| `drivers`       | the driver's constructor id                       | Driver Statistics    |
| `teams`         | the constructor's name, colour and mark           | Driver Statistics    |

- The live boards refresh every 3 seconds while a session is running and every 60 seconds while none is; every other
  resource refreshes every 60 seconds.
- Team logos, nationality flags, driver headshots and the circuit map are fetched from the URLs the payloads name, and
  cached on disk across restarts.

## Parameters

All parameters are manifest-driven widget settings, configurable from the web UI.

| Key          | Name              | Type    | Default          | Purpose                                                             |
| ------------ | ----------------- | ------- | ---------------- | ------------------------------------------------------------------- |
| `view`       | Type of widget    | string  | `auto`           | `auto`, `next_race`, `standings`, or `driver`.                      |
| `local_time` | Use my local time | boolean | `false`          | Show session times in the deck's timezone instead of the circuit's. |
| `driver`     | Driver            | string  | `max_verstappen` | Which driver the Driver Statistics view shows.                      |

The widget follows the deck's localization and timezone settings.

## Constraints

- Rectangular viewports only, from 317×238 up to the Deck's full 1280×480. The layout is chosen from four size buckets
  rather than scaled continuously.
- The driver list is fixed in the manifest, so a mid-season seat change needs a firmware release before the new driver
  can be picked.
- All artwork arrives from Nexus rather than being bundled, so a driver, constructor or country the deployment has no
  image for draws its placeholder.
- Images are fetched and decoded strictly one at a time, so a screenful of them fills over as many round trips as it has
  images. The cache persists across restarts, making this a cold-start cost rather than a recurring one.
- Dates read day-first, as the sport writes a round, except under the `M/D/YYYY` setting.
- No countdown to the next session, and a finished session still reads as "Live" until the next one starts. Both are the
  behaviour of the widget this one replaces, kept deliberately so the port could be compared against it.
