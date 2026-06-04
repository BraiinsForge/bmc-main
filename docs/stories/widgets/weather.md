# Weather Widget

The weather widget displays current conditions and forecast data for a chosen location. It fetches weather data from the
Braiins Forge Nexus weather API, renders condition icons, localized temperatures, forecast bars, sunrise/sunset times,
and full-size wind details across rectangular widget sizes, and can show forecast times in either the location's
timezone or the device timezone.

## User stories

### See current weather at a glance

> As a user, I want the Deck to show the current weather for my chosen location so I can read the conditions without
> opening another device.

- The widget shows the current temperature, weather condition, condition icon, and location name.
- The `small` size focuses on current conditions only.
- The `medium`, `large`, and `full` sizes keep the same current-condition information while adding forecast detail.
- While weather data is still loading, or when no usable data has been received yet because fetches keep failing, it
  reads as `--`; failed fetches retry on their own without user action.

### Choose the forecast location

> As a user, I want to choose the city shown by the weather widget so the forecast is relevant to where I am or where I
> care about.

- The *Location* parameter is a city name; it defaults to `Prague`.
- Changing the location clears the previous weather state and immediately requests fresh weather data.
- If the weather API returns a location miss, the widget shows `Location not found`.
- Empty or whitespace-only locations do not issue a weather request.

### Read the upcoming hours

> As a user, I want to see the next few hours of weather so I can judge near-term conditions at a glance.

- The `medium` size shows the current conditions and the next five hourly forecast entries.
- The `full` size shows the current conditions and the next nine hourly forecast entries.
- Each hourly entry shows the time, condition icon, and temperature.
- The hourly strip starts at the first forecast entry at or after the current weather timestamp, not at the start of the
  day.

### Read today's details

> As a user, I want today's low, high, sunrise, and sunset so I can plan around the day.

- The `large` size shows current conditions, today's low and high temperatures, and today's sunrise and sunset times.
- On shorter Large viewports such as BMM101, the Large layout keeps the same information but moves the temperature and
  condition text to the right of the current-condition icon to save vertical space.
- The `full` size shows sunrise and sunset times alongside the current conditions and hourly strip.
- Times can follow either the weather location's timezone or the device timezone.

### Read the multi-day forecast

> As a user, I want the Deck to show a short forecast so I can compare the next few days quickly.

- The `large` size shows up to four daily forecast rows.
- The `full` size shows up to eight daily forecast rows, split into two columns.
- Each row shows the day label, condition icon, low temperature, high temperature, and a min-to-max range bar.
- Today's row is labelled `Today` and can mark the current temperature on the range bar.

### Use device localization

> As a user, I want weather units and times to match my device settings so the widget reads naturally.

- Temperatures are formatted through the device localization system.
- The `full` size shows wind direction and speed when both values are present in the weather payload. Wind speed follows
  the device unit system: metres per second when metric, miles per hour when imperial.
- Time labels follow the device time format.
- The *Time zone* parameter selects whether forecast times use the weather location's timezone or the device timezone.

## Constraints

- The widget renders at the shared `small`, `medium`, `large`, and `full` sizes on rectangular viewports from 317x238 up
  to 1280x480.
- Non-canonical rectangular viewports keep the closest shared size classification. BMM101's 480x320 fullscreen viewport
  uses the `large` weather layout scaled by the widget fit factor, rather than falling back to `small`.
- The `large` location label uses the full padded widget width, does not wrap, and clips if the API display name is
  still too wide.
- *Location* and *Time zone* are manifest-driven widget parameters, configurable from the web UI.
- The manifest subscribes to the device `localization` and `timezone` settings. There is no separate `units` setting.
- Weather data comes from `https://nexus.braiinsforge.com/api/v1/data/weather/`; the widget polls roughly every 300
  seconds and retries transient failures roughly every 10 seconds.
- A `404` response is treated as a bad location. Other failed or malformed responses retry without turning the state
  into `Location not found`.
- The widget uses `--` as its unavailable-value placeholder.
- Day and night weather icons are chosen from the API's weather code and day/night flags. Unknown weather codes fall
  back to the cloudy icon.
