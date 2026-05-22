# Clock Widget

The clock widget displays the current time on the Deck. It offers an analog or digital face, optional date, seconds, and
timezone readouts, and surfaces the next scheduled alarm.

## User stories

### Choose a clock face style

> As a user, I want to pick how the clock looks so it fits the style of my setup.

- The clock face style is one of: analog with a round dial, analog with a rectangular dial, or digital.
- The digital style shows the time as text; the analog styles show rotating hour, minute, and second hands over a dial.
- Changing the style takes effect without removing the widget or losing its other settings.

### Show or hide the date

> As a user, I want to choose whether the clock shows the date so I can keep the face minimal or informative.

- A *Show date* toggle controls whether the date appears alongside the time.
- The date is a reading row that uses spelled-out month names (e.g. *Mon 3 Mar*), not the device's numeric date format.
  Month/day ordering tracks the device date preference — US (month-first) renders as *Mon, March 3*, every other
  preference renders day-first as *Mon 3 March*. Year-first preferences are treated as day-first because a year-led
  reading row scans as a log line, not a clock face.

### Show or hide seconds

> As a user, I want to choose whether seconds are shown so the clock can tick precisely or stay calm.

- A *Show seconds* toggle controls the seconds readout.
- In the digital style this shows or hides the seconds digits; in the analog styles it shows or hides the second hand.

### Show, hide, or override the timezone

> As a user, I want the clock to show a timezone other than the device's so I can track time in another location.

- A *Show timezone* toggle controls whether a timezone label is shown.
- The label shows the city and its UTC offset.
- By default the clock follows the device's system timezone.
- An optional timezone override makes the clock display a chosen IANA timezone instead.
- An unresolvable timezone is shown as unknown rather than as a wrong time.

### Choose the numeral weight

> As a user, I want to adjust the weight of the clock's numbers so they read well on my display.

- The numeral and digit font weight is one of: regular, semi-bold, or bold.

### See the next alarm

> As a user, I want the clock to show my next alarm so I know it is set without opening settings.

- When an alarm is scheduled, the clock shows a bell icon next to the next alarm time.
- The alarm row is shown only at the `full` size; smaller sizes omit it to keep the clock face legible.
- When no alarm is scheduled, no alarm row is shown.
- The alarm time follows the device's 12- or 24-hour format.

## Constraints

- The widget renders at the shared `small`, `medium`, `large`, and `full` sizes; layout and detail adapt per size.
- Clock style, date, seconds, timezone, numeral weight, and the timezone override are manifest-driven widget parameters,
  configurable from the web UI.
- Day / night appearance follows the device-wide night mode signal — the clock recolours itself; it is not a per-widget
  setting. See [Night Mode](night-mode.md).
- The next alarm and the 12-/24-hour time format come from device system state, not from widget configuration.
