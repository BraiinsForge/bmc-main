# Mining Status Indicator

On a miner the BMC application shows a small pickaxe in the bottom-right corner of the display whenever the miner needs
a look: violet while the miner tunes, red while it underperforms, is stopped, or cannot be reached. A miner that mines
normally shows nothing, so an empty corner means all is well.

## User stories

### See that the miner is tuning

> As a BMM101 user, I want to see when my miner is tuning so that I know a hashrate below nominal is expected and not a
> fault.

- The pickaxe is violet while the tuner is preheating, tuning, or continuously tuning and at least one board is hashing.
- It disappears once tuning finishes and the boards reach their nominal rate.
- A miner paused or stopped in the middle of a tuning run stays violet until its one-minute hashrate drains to zero,
  then turns red. That takes about a minute.

### Notice when the miner underperforms or stops

> As a BMM101 user, I want a red pickaxe when my miner is not mining as it should so that I notice a problem without
> opening the miner's web UI.

- The pickaxe is red when any active board's five-minute hashrate is under 80 % of its nominal, or when a board reports
  no five-minute rate at all.
- It is red when no board is active: every board disabled or never clocked, or the miner still starting (cold wait,
  cooling down, delayed start, defrosting).
- A dead pool, a pause, or a stop for any reason (user, thermal, license, tuner or hardware error) turns it red within
  about a minute, as the hashrate means drain. The indicator shows that the miner stopped hashing, not why.
- Boards the user disabled are ignored; the remaining boards decide, so a board left off never holds the corner red.
- The indicator keeps its last state through five consecutive failed readings and then turns red, so a brief hiccup in
  the miner's API does not flicker the corner. Stopping Boser turns it red the same way, and restarting Boser recovers
  it on the first successful reading. If the miner has never answered since the BMC application started, nothing is
  shown until those five readings have failed.

### Keep the corner empty while mining normally

> As a BMM101 user, I want no indicator while my miner mines normally so that the display stays uncluttered.

- No pickaxe is drawn while the tuner reports no tuning stage, at least one board is active, and every active board is
  at or above 80 % of its nominal. There is no positive "mining normally" indicator, on purpose.
- Every transition between tuning, normal, low, and unreachable updates the corner as soon as it is read, including
  removing the pickaxe when the miner becomes healthy.

### Tell a network problem from a mining problem

> As a BMM101 user, I want the network and mining indications kept apart so that an offline device does not read as a
> broken miner.

- While the device has no network connectivity, the OFFLINE chip takes the corner and the pickaxe is hidden.
- The mining status keeps updating in the background, so the pickaxe comes back in its current state as soon as
  connectivity returns.

## Constraints

- The indicator exists only on products with a mining function (BMM100, BMM101, BFM100) and is designed for the BMM101
  display. On a BMC100 Deck the corner shows only the OFFLINE chip.
- The pickaxe sits on a small translucent card of the same height and background as the OFFLINE chip, flush with the
  bottom-right corner, so the two read as one indicator changing content. It obscures no other persistent UI, ignores
  touch, and every other on-screen element (a firing alarm, the upgrade screens, the settings tray) draws over it.
- The status is derived from the tuner state and the boards' hashrates only. Pool liveness and the license are not
  checked; a limited or expired license shows only if the miner stops hashing, which the hashrate reports about a minute
  late.
- The BMC application reads the miner's local API every five seconds as the bearer of the local API token Boser issues
  to on-device clients, so setting or changing the miner password never affects the indicator. A Boser restart issues a
  new token; the indicator picks it up on its next reading. While Boser is down and the token is unavailable, the
  readings fail and the five-failed-readings rule above applies.
