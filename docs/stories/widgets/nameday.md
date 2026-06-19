# Nameday Widget

The nameday widget displays the name or names celebrated today in a chosen country. It shows a country header with a
flag, today's names as a large headline, and an optional date readout. It fetches the day's names from a public nameday
API and refreshes once a day at local midnight.

Implementation note: Currently we also check data validity each minute: if the device's time is incorrect on boot (e.g.,
RTC backup drained), next retry will be scheduled incorrectly.

## User stories

### See today's namedays

> As a user, I want the Deck to show whose nameday it is today so I can see at a glance who to congratulate.

- The widget shows the names celebrated today as a large, centered headline.
- A header above the names shows the selected country's flag and name.
- The names refresh automatically at local midnight, so the face always reflects the current day.
- While the names are still loading — including when no value has been received yet because fetches keep failing — the
  headline reads `Loading...`; failed fetches retry on their own without user action.
- When the day's data has loaded but the selected country has no nameday today, the headline reads `N/A`.

### Choose the country

> As a user, I want to pick which country's namedays are shown so the widget matches the calendar I follow.

- A *Country* parameter selects the country; it defaults to `Czechia`.
- The available countries are Austria, Czechia, Germany, Denmark, Estonia, Spain, Finland, France, Croatia, Hungary,
  Italy, Lithuania, Latvia, Poland, Sweden, Slovakia, and the United States.
- The header flag and label update to match the selected country, and the names shown are those celebrated for that
  country today.
- Changing the country immediately requests fresh names for the new selection.

### Show or hide the date

> As a user, I want to choose whether the widget shows today's date so I can keep the face minimal or informative.

- A *Show Date* toggle controls whether the current date appears below the names; it is on by default.
- The date follows the device's configured date format. The timezone used for the date is the one in effect when the
  names were last fetched, so a timezone change is reflected on the next refresh rather than instantly.
- When the date is unavailable, it reads as `--` rather than as a wrong date.

### Stay correct across timezone changes

> As a user, I want the widget to show the right day's names wherever the Deck is set, so a timezone change does not
> leave it on yesterday's or tomorrow's names.

- The day used for the lookup follows the device's configured timezone.
- Changing the device timezone re-requests the names when it moves the widget onto a different local day; the date
  readout then also picks up the new timezone.

## Constraints

- The widget renders at the shared `small`, `medium`, `large`, and `full` sizes; the names headline and the date scale
  per size.
- *Country* and *Show Date* are manifest-driven widget parameters, configurable from the web UI.
- The date format and timezone follow the device's localization and timezone settings — they are not per-widget
  settings.
- The names headline wraps to fit. Long lists of names are truncated with a trailing ellipsis (`...`), with a higher
  character budget at the `full` size than at the smaller sizes.
- Nameday data comes from the public nameday API at `nameday.abalin.net`. The widget requests the names for the current
  local day and month, refreshes at the next local midnight, and retries failed or unparseable fetches roughly every 10
  seconds.
- The widget shows `Loading...` until the first successful fetch, `N/A` when the selected country has no nameday today,
  and `--` when the date is unavailable.
- The widget renders on rectangular viewports from 317x238 up to 1280x480.
