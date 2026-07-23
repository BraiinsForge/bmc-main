# Fleet Management Widget

The fleet management widget gives an at-a-glance view of every Bitcoin miner on the local network, and lets the operator
drill from the whole-fleet overview down to a single miner. It discovers miners over mDNS, polls each one for live
telemetry, and rolls the readings up into a fleet total, a per-model breakdown, and a per-device detail view. It
supports three device families — BOS, Braiins OS Libre, and AxeOS (Bitaxe / NerdQAxe++) — each independently discovered,
authenticated, and polled, so one family failing never blanks another's numbers.

## User stories

### See my whole fleet at a glance

> As a miner operator, I want one screen that sums up every miner on my network so I can confirm the fleet is healthy
> without opening each device.

- The overview shows the fleet total: combined hashrate, power, and efficiency, a min/avg/max temperature spread, and a
  status breakdown of how many miners are OK, degraded, or off.
- A hashrate chart carries the fleet's recent trend and headline hashrate (see *Read the hashrate trend*).
- Efficiency is total power divided by total hashrate, not the mean of per-device ratios. Only actively-mining devices
  count toward it, so an idle-but-powered miner is left out of the ratio rather than inflating it — though its draw
  still counts in the fleet's total power.
- Temperature reads as a `min/avg/max °C` spread over the fleet; a single-sensor device collapses the three to one.

### Know which miners are healthy

> As an operator, I want each miner classified as healthy, underperforming, or down so I can spot trouble without
> reading raw numbers.

- Every reported miner is in one of three states: **OK** (reachable and hashing at or above its expected rate),
  **Degraded** (reachable but underperforming or idle), or **Off** (not responding).
- A miner is OK when its current hashrate is at least 20% of its nominal (nameplate) hashrate. The nominal comes from
  the miner's own API where it exposes one, otherwise from a built-in model catalog; with neither known, a small
  hashrate floor stands in.
- A device's detail screen splits *off* by cause: unreachable (no HTTP response at all), API error (the device answered,
  but with an error such as `503`), or not authenticating (the device answered but rejected the login — a prompt to
  check the credentials).

### Move between the overview and a per-model list

> As an operator, I want to flip between the fleet dashboard and a detailed per-model table.

- A grid/list toggle switches between the dashboard overview (grid) and the per-model breakdown table (list).

### Break the fleet down by model

> As an operator running mixed hardware, I want miners grouped by model so I can compare how each type is performing.

- The list view shows one row per resolved model name, each with that model's hashrate, a hashrate sparkline, power,
  efficiency, average temperature, and its OK/degraded/off counts.
- Groups are ordered by family — Braiins OS Libre first, then BOS, then Bitaxe — and alphabetically by model name within
  a family.
- Miners whose model cannot be resolved collect into a single *Unknown* group, pinned last.
- The table pages when the model list is longer than the body height.

### Drill into a model's devices

> As an operator, I want to open a model and see each individual miner of that type.

- Opening a model's *Detail* shows a per-device list for that model: one row per miner with its hostname, hashrate and
  sparkline, power, efficiency, and avg/min/max temperature.
- A miner with no live telemetry shows its status (unreachable / API error) where the metric columns would be, instead
  of a row of meaningless zeros.
- The list pages when it exceeds the body height; a back affordance returns to the fleet.

### Inspect a single miner

> As an operator, I want a full read-out for one miner so I can diagnose it.

- Opening a device's *Detail* shows its detail screen: IP and MAC, state, current hashrate with a chart, nominal
  (nameplate) hashrate, power, efficiency, uptime, and temperature (a single value or avg/min/max).
- The nominal hashrate is a static nameplate value, not a time series.

### Read the hashrate trend

> As an operator, I want to see how hashrate has moved recently, not just the current number.

- The dashboard and device-detail hashrate charts plot recent history on a 0-anchored scale — topped at the nominal
  hashrate where it is known — with subtle gridlines and, on the dashboard, a value scale, so a small fluctuation reads
  as small rather than filling the whole chart.
- The per-model and per-device rows carry a compact sparkline of the same history.
- History is accumulated on-device from successive polls, so it is uniform across families whether or not a miner
  exposes a hashrate-history endpoint.

### Point the widget at my miners' credentials

> As an operator, I want to give the widget one set of credentials per family so it can read stats from every miner of
> that family on the network.

- *BOS password* is the `root` password used to log into every BOS miner; the username is always `root`.
- *Braiins OS Libre username* and *Braiins OS Libre password* are the HTTP Basic credentials used against every Braiins
  OS Libre device.
- AxeOS miners need no credentials.
- Credentials are shared fleet-wide per family — one BOS password and one Braiins OS Libre login for the whole network,
  not a per-device setting.

### Show or hide AxeOS miners

> As an operator, I want to hide the AxeOS (Bitaxe / NerdQAxe++) miners so they do not clutter a view of my main fleet.

- *Show AxeOS miners* toggles the AxeOS family in the view; turning it off hides every AxeOS device and stops polling
  them. BOS and Braiins OS Libre are always shown.
- mDNS discovery keeps running while AxeOS is hidden, so re-enabling it resumes polling the already-discovered devices
  without re-discovery.

### Choose the hashrate chart's time range

> As an operator, I want to widen or narrow the window the hashrate charts cover.

- *Chart time range* sets the span the dashboard and device-detail hashrate charts cover — 15 minutes, 1 hour (the
  default), 6 hours, or 24 hours. A fixed number of points spans the range, so a longer range samples less often.

### Name my fleet

> As an operator, I want a heading on the widget so I can tell which fleet I am looking at.

- *Fleet name* is the heading shown above the fleet overview; it defaults to *My Fleet*.

### Trust what the numbers say

> As an operator, I want clear behavior when miners come and go so the screen never shows a stale value as if it were
> live, nor a phantom fleet after everything is gone.

- A miner keeps its last good reading through a few failed passes before it flips to unreachable, so a single missed
  poll on a flaky network does not blank it. Once unreachable it counts as *off* and drops out of the fleet totals
  entirely — an unreachable miner is unknown, not a measured zero, so an all-down group reads as unavailable rather than
  a fabricated `0`.
- A confirmed miner is kept across an mDNS *removed* event — which fires unreliably on lossy Wi-Fi as a cache expiry,
  not only a real departure — so a dropped announcement cannot churn it out of the fleet. Its liveness is governed by
  polling from then on, not by discovery.
- A miner that has had no HTTP response for several minutes is retired from the fleet, so a genuinely dead fleet's count
  decays toward zero rather than freezing on the last-known members. A miner that is present but API-erroring (answering
  with an error) is kept, not retired.
- Failed and timed-out fetches retry on the next pass on their own; the widget keeps the last good data between passes.
- AxeOS reports `-1` for a sensor it has not read yet (notably right after boot); the widget drops these negative
  sentinels so they never pollute the fleet totals.
- Numbers use the device's configured number format for digit grouping and the decimal mark.
- Until the first miner answers, the widget shows *Searching for miners…*.

## Supported families

| Family           | Display label    | mDNS browse  | Default port | API base      | Auth                                                                   |
| ---------------- | ---------------- | ------------ | ------------ | ------------- | ---------------------------------------------------------------------- |
| BOS              | BOS              | `_http._tcp` | 80           | `/api/v1`     | token login at `/auth/login` (`root` + *BOS password*)                 |
| Braiins OS Libre | Braiins OS Libre | `_ubos._tcp` | 8080         | `/api`        | HTTP Basic (*Braiins OS Libre username* / *Braiins OS Libre password*) |
| AxeOS            | Bitaxe           | `_http._tcp` | 80           | `/api/system` | none                                                                   |

- **Discovery.** The widget runs two mDNS browses: the base `_http._tcp` service (BOS and AxeOS share it) and Braiins OS
  Libre's own `_ubos._tcp`. On `_http._tcp`, AxeOS is identified up front by its discovery TXT records; a BOS miner
  carries no distinguishing signal there, so it enters as a *candidate* and is only admitted to the report once it
  answers a poll — a non-miner web server is probed a few times and then dropped. Braiins OS Libre is identified
  directly by its own service type. A miner that advertises neither browsed type, or sits on a network segment mDNS does
  not reach, is not discovered.
- **BOS** (Braiins OS) miners are polled across three endpoints: `/miner/stats` (hashrate and power),
  `/miner/hw/hashboards` (the hottest chip temperature and the chip type/count), and `/miner/details` (uptime, platform,
  nominal hashrate, and miner model). A `401`/`403` triggers one re-authentication per device per pass.
- **Braiins OS Libre** devices expose a single `/info` endpoint carrying hashrate, power, temperature, and uptime.
  Braiins OS Libre advertises no platform identifier and no nominal hashrate, so its product name doubles as the model
  grouping and its nameplate hashrate comes from the built-in model catalog.
- **AxeOS** miners — Bitaxe and NerdQAxe++ running ESP-Miner — expose a single `/info` endpoint carrying hashrate,
  power, temperature, uptime, and nominal hashrate. The model is resolved from `deviceModel` (falling back to the board
  version), with chip type and count from `ASICModel`/`asicCount`; discovery TXT records also seed a provisional model
  hint before the first poll.

## Parameters

All parameters are manifest-driven widget settings, configurable from the web UI.

| Key                  | Name                      | Type    | Default    | Purpose                                                              |
| -------------------- | ------------------------- | ------- | ---------- | -------------------------------------------------------------------- |
| `fleet_name`         | Fleet name                | string  | `My Fleet` | Heading shown above the fleet overview.                              |
| `bos_password`       | BOS password              | string  | `root`     | Root password used to log into every BOS miner on the network.       |
| `ubos_username`      | Braiins OS Libre username | string  | `root`     | User name for HTTP Basic auth against every Braiins OS Libre device. |
| `ubos_password`      | Braiins OS Libre password | string  | `root`     | Password for HTTP Basic auth against every Braiins OS Libre device.  |
| `axeos_enabled`      | Show AxeOS miners         | boolean | `true`     | Include AxeOS miners in the view and keep polling them.              |
| `chart_span_minutes` | Chart time range          | integer | `60`       | Minutes the hashrate charts cover: `15`, `60`, `360`, or `1440`.     |

## Constraints

- The widget targets the Deck's full **1280×480** rectangular viewport only; the manifest declares no smaller sizes.
- Font sizes are fixed for that layout.
- Every device across all families is polled on a single global round-robin, one device at a time, aiming to refresh the
  whole fleet roughly every 5 seconds — bounded by a floor on the per-device poll rate, so a large fleet's freshness
  degrades gracefully rather than the box drowning in parallel requests.
- The *BOS password*, *Braiins OS Libre username*, and *Braiins OS Libre password* are stored and shown as ordinary
  widget text (the manifest system has no secret-parameter type yet) and are sent unencrypted over the local network,
  since the miner APIs are HTTP-only. This is a known limitation.
- Credentials are shared fleet-wide per family; the widget cannot use different credentials for individual miners of the
  same family.
- Number formatting follows the device's localization system setting; it is not a per-widget setting.
- mDNS discovery is subject to the host runtime's browse limits; if a browse is rejected the widget logs a warning and
  that family is not discovered.
