# ISS Position Widget

The ISS position widget tracks the International Space Station live on the Deck. At full size it renders a 3D globe with
the station's marker, its orbital ground track, and the day/night terminator; at smaller sizes it shows a position and
telemetry panel. Data comes from the Braiins nexus service and refreshes about every 30 minutes, while the live position
is propagated on-device between refreshes so the marker keeps moving.

## User stories

### See where the ISS is right now

> As a user, I want the Deck to show the current position of the ISS so I can see where it is over Earth at a glance.

- At full size, the station appears as a marker on a 3D globe centred on its current sub-point, with the orbital ground
  track drawn ahead of and behind it and a day/night terminator shading the night side.
- At the large, medium, and small sizes — and at full size when no orbital data is available — the widget shows a panel
  with the ground position the ISS is currently over.
- Between refreshes the position is propagated on-device, so the marker moves smoothly rather than jumping once per
  refresh.

### Read the station's telemetry

> As a user, I want to see the ISS's altitude, speed, and whether it is in sunlight so I get more than a dot on a map.

- The panel reads out the altitude, the orbital velocity, and the visibility — Sunlit or Eclipsed — for the current
  position.

## Constraints

- The widget renders on rectangular viewports only, from 317×238 up to 1280×480 (the shared small, medium, large, and
  full sizes); it is not offered on round panels.
- The 3D globe with the orbital track renders only at full size and only when orbital elements are available; the other
  sizes, and full size without orbital data, show the position/telemetry panel.
- It has no per-widget settings — there is nothing for the user to configure.
- Position and orbital data come from the Braiins nexus service; the widget refreshes about every 30 minutes and
  propagates the live position locally in between. A failed refresh keeps the last known position on screen; an error is
  shown only if no position has ever been received.
