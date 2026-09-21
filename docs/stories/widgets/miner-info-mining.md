# Miner Info — Mining Widget

A miner overview: the readings that say whether one Bitcoin miner is running healthily. It reads a BOS miner over its
REST API and nothing else, so it needs no internet connection.

Add the widget once per miner to watch several at a time.

## User stories

### See my miner at a glance

> As a user, I want a quick overview of my miner so I can confirm it is running healthily.

- The widget shows current hashrate (TH/s), temperature (°C), power consumption (W), MCR (%), fan speed (%), and the
  miner's IP address.
- Temperature reads as a board-to-chip range (e.g. *61-74*), matching the BOSer miner screen.
- Rows fill the height of the display rather than packing at the top. On the BMM101 they sit under the widget's icon and
  name with hairlines between, at the frame's size.

### See my miner on a round display

> As a user with a round-display Deck, I want the overview to fill the circular screen and show miner health at a
> glance.

- On the round 480×480 display (BFM100) the current hashrate sits at the centre of a 28-segment ring, with the *TH/s*
  unit trailing to its right, and four stats occupy the quadrants around it: power consumption, MCR, temperature, and
  fan speed.
- In those compact clusters temperature reads as the chip temperature alone, not the board-to-chip range the rectangular
  screen shows.
- Above the ring a chip header shows a chip icon, the chip model and the count across all hashboards (e.g. *BM1370
  x108*). It appears only when the miner reports both; otherwise it is omitted rather than showing placeholders.

### Read the miner's health from the ring

> As a user, I want the ring to tell me whether the miner is running as tuned so I can judge it without reading numbers.

- The ring's fill reflects hashrate against the miner's configured tuner targets, anchored at three points — the minimum
  target a quarter of the way around, the default at three-quarters, and the maximum at the full ring — interpolating
  linearly between, so a miner at its default target fills about three-quarters of it.
- The ring's colour compares hashrate to the **default** target: green within a small tolerance of it (currently ±5%),
  purple at least that far above, amber at least that far below, and red with a single lit tick when the miner is not
  hashing.
- When the hashrate or its target is unavailable the ring stays gray and unlit, and the hashrate label reads neutral
  rather than implying a state.
- The MCR shown in the quadrant is a separate readout and does not drive the ring.

### Point the widget at my miner

> As a user, I want the widget to read my own miner without a password, and to be able to watch another miner on my
> network by giving its address and login.

- The widget has two account slots and uses exactly one of them:
  - *Local BOS token*: the miner this display belongs to. Bind the *Local BOS API* account (present from first boot on
    BFM100 and BMM) and keep the *Miner URL* at `http://localhost/api/v1`. No password is entered, and the widget keeps
    working after the miner password changes.
  - *Remote miner login*: a miner elsewhere on the network. Set the *Miner URL* to its API base, for example
    `http://10.0.0.5/api/v1`, and bind an account holding that miner's password; the username is always `root`. The
    widget logs in, caches the session token, and re-authenticates on its own when it expires.
- With neither slot bound the widget shows "Bind a BOS account"; with both it shows "Bind one BOS account, not both".
  The local token is never sent to the *Miner URL*.
- A remote username or password containing `"` or `\` is not supported.
- Stats refresh roughly every five seconds. Pointing the widget at a different miner clears the readings first, so one
  miner's figures are never shown under another's address.

### Trust what the numbers say

> As a user, I want clear placeholders when data is missing so I never mistake a stale or absent value for a real one.

- Unavailable values read as `N/A`.
- Credentials the miner turns away show a `Cannot authenticate` banner over the fields: a refused login in remote mode,
  a rejected token in local mode, where there is no login to refuse.
- A miner that never answers, or that answers but fails its telemetry, shows `Failed to load: Miner` instead.
- Failed fetches retry on their own without user action. When a refresh keeps failing the last good values stay on
  screen under a `Stale data` banner until the next successful fetch.
- Numbers use the device's configured number format for digit grouping and the decimal mark.

## Constraints

- The widget renders on rectangular viewports from 317×238 up to 480×320 and on the round 480×480 viewport. The
  rectangular targets are the BMM100 (320×240), the BMM101 (480×320), and the BMC100's 1×1 slot (317×238); the round
  viewport targets the BFM100.
- The wider BMC100 views are deliberately unsupported: the layout is drawn for a 480-wide screen and a design for the
  larger ones does not exist yet.
- Font sizes are fixed across viewports — fields are hidden rather than shrunk.
- *Miner URL* is a manifest-driven widget parameter; the two account slots are credential bindings, chosen from saved
  accounts in the web UI. No password is stored in widget parameters.
- Number formatting follows the device's localization system setting; it is not a per-widget setting.
- Field sets, labels, and units mirror the BOSer BMM screens.
- The tuner constraints that scale the ring are read from `/configuration/constraints`. They are fetched only on the
  round viewport, and only once per login, since they change only when the miner is re-tuned.
