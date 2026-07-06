# Random Facts Widget

The random facts widget shows a single random factoid, large and centered on the tile, under a fixed "Random Facts"
header. It fetches a fresh fact from a public facts API and rotates to a new one every few minutes.

## User stories

### See a random fact

> As a user, I want the Deck to show an interesting random fact so I always have something new to read at a glance.

- The widget shows a static "Random Facts" header at the top and the current fact as a large, centered block of text.
- The fact text wraps across lines and auto-fits the available area, so both short and long facts fill the tile without
  overflowing.
- A fresh fact is fetched automatically every few minutes; no user action is needed.
- While the first fact is still loading — including when no fact has been received yet because fetches keep failing —
  the tile reads `Loading...`.
- Once a fact has been shown, it stays on screen if a later refresh fails; failed fetches retry on their own until a new
  fact arrives.

## Constraints

- The widget has no configurable parameters.
- The widget renders at the shared `small`, `medium`, `large`, and `full` sizes; the header and the fact text scale per
  size.
- Fact data comes from the public useless-facts API at `api.viewbits.com`; the widget polls roughly every five minutes
  and retries failed or unusable fetches on its own.
- Only the fact text from the response is shown; the source and URL attribution the API returns are ignored.
- The widget re-renders on a theme change without re-spawning; no other system settings affect it.
- The widget renders on rectangular viewports from 317x238 up to 1280x480.
