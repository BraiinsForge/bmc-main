# Formula 1 capture fixtures

What the recorded set covers, and why it is shaped this way. `config.toml` is the machine-readable list; a fixture is
named `<platform>-<viewport>-<scenario>`.

## How a fixture is made

`just widgets::formula-1::run-netsim` serves every scenario at once, one port each, from `../sim-blueprint.json5`.
`just widgets::formula-1::record <scenario>` then opens a testbed aimed at that scenario's port.

Three levels. A **launch** is one testbed against one profile. Inside it, a **take** is one `device:size`, recorded and
saved before pressing Record again for the next. Inside a take, **captures** walk the params: each `view` change lands
as a `ParamDelivery` and each Capture as a frame, so one take covers every view that size can show.

Auto-capture is on, so switching a view already appends its frame. Pressing Capture after a switch writes a second,
byte-identical one, and every duplicate becomes a baseline CI diffs forever. Capture by hand only while a board is
running.

The takes were recorded with **System** at its defaults — Europe/Prague, Hour24, DD.MM.YYYY, Metric. A baseline pins
whatever it was taken under.

## Sweeping a live board

The live profiles run at `time_scale: 5`, so a lap lands every ~14 s, and each cycles a fixed `lap_window` that holds
the state it was pinned to. Capture whenever the render changes: the grid closing up, a car entering and leaving the
pits, a sector turning purple are what a live board exists to draw, and a single frame catches none of them.

| Profile    | window | one sweep |
| ---------- | ------ | --------- |
| `race`     | 7 laps | ~101 s    |
| `sprint`   | 5 laps | ~72 s     |
| `practice` | 4 laps | ~58 s     |
| `quali`    | 2 laps | ~29 s     |

That column is how long to sit there. A sweep cut to a few seconds photographs one state repeatedly; where the window
restarts inside it does not matter, since every frame replays from its own recorded reply.

## The inventory

Viewports are `bmc100` `full` 1280×480, `large` 638×480, `medium` 638×238 and `small` 317×238, plus `bmm100:full`
320×240 and `bmm101:full` 480×320. The layout bucket follows width alone, so both BMM panels land in the same bucket as
`bmc100:small`.

| Scenario     | Targets                                              | Views walked                 | What it pins                                                         |
| ------------ | ---------------------------------------------------- | ---------------------------- | -------------------------------------------------------------------- |
| `race`       | `bmc100` full, large, medium, small; both BMM panels | all four                     | the race board, and every screen that does not vary by scenario      |
| `sprint`     | `bmc100` full, large                                 | Automatic, Next Race         | the sprint schedule, two practices traded for a sprint and its quali |
| `quali`      | `bmc100` full, large, medium; `bmm100:full`          | Automatic                    | split seating at full size, and where the sectors drop out below it  |
| `practice`   | `bmc100` full, large                                 | Automatic                    | `OUT LAP` in the gap and lap columns, `LEADER` on P1                 |
| `idle`       | `bmc100` full, small; both BMM panels                | Automatic                    | the next race's card, reached down the chain rather than chosen      |
| `off-season` | `bmc100:full`                                        | Driver Standings, Next Race  | both empty states: `Standings unavailable`, `Next race unavailable`  |
| `cold-start` | `bmc100:full`                                        | Driver Statistics            | the careers 503 trio at once — statistics table, driver index, card  |
| `warming`    | `bmc100:full`                                        | Driver Statistics, both ends | the same trio recovering a minute in; the transition is the point    |

Standings and Driver Statistics do not vary by scenario while their data is there, so their per-size coverage comes from
the `race` launch. Every other launch walks a view only where its own scenario changes it: `off-season` for the empty
standings, `cold-start` and `warming` for the 503 trio.

## What the set does not cover

- **`bmc100:small` on `sprint`.** Small sets `Schedule::Absent`, so it never draws the session column a sprint weekend
  changes; its card hashes byte-identical to the `race` take's.
- **`Next Race` inside `idle`.** With nothing running, `Automatic` walks its chain past all three boards to the next
  race's card — the same screen `Next Race` selects outright, to the byte. That the chain lands there is pinned by
  `select_screen`'s own test, which says so in words a diff cannot.
- **`stale` (port 20106).** `stale`, `cache_age` and `ttl` appear nowhere in `../src`: the widget draws an envelope's
  data and never reads how old the server calls it. The profile also leaves `session` at its `Idle` default, so its
  frame would be `idle`'s. Holding last-good data past a failure is a real behaviour worth a fixture, but it wants a
  good reply followed by a failing one — `status`, not `stale_secs`.
- **`race-eve` (port 20109).** Uncovered. It pins a Thursday, the one profile that can show a weekend about to open,
  which makes it the obvious next fixture.
- **`local_time` and `unit_system: Imperial`.** Both change rendering and no take sets either.
- **BFM100.** Round, and the manifest admits rectangular viewports only.
