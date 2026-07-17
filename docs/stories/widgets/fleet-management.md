# Fleet Management Widget

The fleet management widget gives an at-a-glance view of every Bitcoin miner on the local network, and lets the operator
drill from the whole-fleet overview down to a single miner. It discovers miners over mDNS, polls each one for live
telemetry, and rolls the readings up into a fleet total, a per-model breakdown, and a per-device detail view. It
supports three device families — BOS, uBOS, and AxeOS (Bitaxe / NerdQAxe++) — and each family is independently
discovered, authenticated, polled, and toggled, so one family failing or being disabled never blanks another's numbers.

## User stories

### See my whole fleet at a glance

> As a miner operator, I want one screen that sums up every miner on my network so I can confirm the fleet is healthy
> without opening each device.

- The overview shows the fleet total: combined hashrate, power, and efficiency, a min/avg/max temperature spread, and a
  status breakdown of how many miners are OK, degraded, or off.
- A hashrate chart carries the fleet's recent trend and headline hashrate (see *Read the hashrate trend*).
- Efficiency is total power divided by total hashrate across the fleet, not the mean of per-device ratios, so an idle
  but powered miner correctly drags the figure down.
- Temperature reads as a `min/avg/max °C` spread over the fleet; a single-sensor device collapses the three to one.

### Know which miners are healthy

> As an operator, I want each miner classified as healthy, underperforming, or down so I can spot trouble without
> reading raw numbers.

- Every reported miner is in one of three states: **OK** (reachable and hashing at or above its expected rate),
  **Degraded** (reachable but underperforming or idle), or **Off** (not responding).
- A miner is OK when its current hashrate is at least 20% of its nominal (nameplate) hashrate. The nominal comes from
  the miner's own API where it exposes one, otherwise from a built-in model catalog; with neither known, a small
  hashrate floor stands in.
- A device's detail screen splits *off* by cause: unreachable (no HTTP response at all) or API error (the device
  answered, but with an error such as `503`).

### Move between the overview and a per-model list

> As an operator, I want to flip between the fleet dashboard and a detailed per-model table.

- A grid/list toggle switches between the dashboard overview (grid) and the per-model breakdown table (list).

### Break the fleet down by model

> As an operator running mixed hardware, I want miners grouped by model so I can compare how each type is performing.

- The list view shows one row per resolved model name, each with that model's hashrate, a hashrate sparkline, power,
  efficiency, average temperature, and its OK/degraded/off counts.
- Groups are ordered by family — uBOS first, then BOS, then Bitaxe — and alphabetically by model name within a family.
- Miners whose model cannot be resolved collect into a single *Unknown* group, pinned last.
- The table pages when the model list is longer than the body height.

### Drill into a model's devices

> As an operator, I want to open a model and see each individual miner of that type.

- Opening a model's *Detail* shows a per-device list for that model: one row per miner with its hostname (or friendly
  name), hashrate and sparkline, power, efficiency, and avg/min/max temperature.
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

### Give devices friendly names

> As an operator, I want to label miners by where they are so the per-device list reads at a glance.

- *Device names* is a JSON object mapping a device's mDNS name — or a manual host/IP — to a friendly label shown in the
  per-model device list (e.g. `{"miner-a": "Rack 3 left", "10.0.0.5": "Test bench"}`).
- The key is the device's mDNS name before the first `._`, or the host/IP as entered in *hosts* (a `:port` suffix is not
  part of the key).

### Point the widget at my miners' credentials

> As an operator, I want to give the widget one set of credentials per family so it can read stats from every miner of
> that family on the network.

- *BOS password* is the `root` password used to log into every BOS miner; the username is always `root`.
- *uBOS username* and *uBOS password* are the HTTP Basic credentials used against every uBOS device.
- AxeOS miners need no credentials.
- Credentials are shared fleet-wide per family — one BOS password and one uBOS login for the whole network, not a
  per-device setting.

### Reach miners that mDNS does not find

> As an operator on a segmented or quiet network, I want to name miners by address so the widget polls them even when
> mDNS discovery does not surface them.

- *BOS hosts*, *uBOS hosts*, and *AxeOS hosts* are each a JSON array of extra hosts to poll, beyond mDNS discovery.
- Each entry is `host` or `host:port` (e.g. `["10.0.0.5", "miner.local:8080"]`); a bare host uses the family's default
  port (80 for BOS and AxeOS, 8080 for uBOS). IPv6 literals use bracket notation with an explicit port (e.g.
  `[fe80::1]:80`).
- A host found both manually and via mDNS appears twice in the fleet.
- Editing a host param reconciles the manual set live: added hosts start polling, removed hosts stop and drop their
  cached session token. Clearing a family's manual hosts requires the explicit empty array `[]`; invalid JSON or any
  other empty value leaves the manual set unchanged, so a typo cannot silently wipe the list.

### Choose which models the widget shows

> As an operator, I want to filter the fleet view down to the models I care about, or hide ones I do not.

- *Shown models* (whitelist) is a JSON array of model-name fragments; when non-empty, only matching models are shown.
  *Hidden models* (blacklist) is a JSON array of fragments to hide.
- Matching is case-insensitive and whitespace-insensitive, against both the model name and its internal id (e.g.
  `["bmm101"]` matches `Braiins Mini Miner BMM 101`).
- The blacklist overrides the whitelist for the same model. A device whose model has not yet resolved cannot be filtered
  by model and stays visible.
- Filtered-out devices are removed from both their breakdown group and the fleet total.

### Turn whole families on and off

> As an operator, I want to switch a device family off so the widget ignores it entirely.

- *Show BOS miners*, *Show uBOS miners*, and *Show AxeOS miners* each include their family in the view and keep polling
  it; turning one off hides every device of that family and stops polling them.
- mDNS discovery keeps running for a disabled family, so re-enabling it resumes polling the already-discovered devices
  without re-discovery.

### Name my fleet

> As an operator, I want a heading on the widget so I can tell which fleet I am looking at.

- *Fleet name* is the heading shown above the fleet overview; it defaults to *My Fleet*.

### Trust what the numbers say

> As an operator, I want clear behavior when miners come and go so the screen never shows a stale value as if it were
> live, nor a phantom fleet after everything is gone.

- A miner keeps its last good reading through a few failed passes before it flips to unreachable, so a single missed
  poll on a flaky network does not blank it. Once unreachable it folds into the totals as zero and counts as *off*.
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

| Family | Display label | mDNS browse  | Default port | API base      | Auth                                                   |
| ------ | ------------- | ------------ | ------------ | ------------- | ------------------------------------------------------ |
| BOS    | BOS           | `_http._tcp` | 80           | `/api/v1`     | token login at `/auth/login` (`root` + *BOS password*) |
| uBOS   | uBOS          | `_ubos._tcp` | 8080         | `/api`        | HTTP Basic (*uBOS username* / *uBOS password*)         |
| AxeOS  | Bitaxe        | `_http._tcp` | 80           | `/api/system` | none                                                   |

- **Discovery.** The widget runs two mDNS browses: the base `_http._tcp` service (BOS and AxeOS share it) and uBOS's own
  `_ubos._tcp`. On `_http._tcp`, AxeOS is identified up front by its discovery TXT records; a BOS miner carries no
  distinguishing signal there, so it enters as a *candidate* and is only admitted to the report once it answers a poll —
  a non-miner web server is probed a few times and then dropped. uBOS is identified directly by its own service type. A
  miner that advertises neither browsed type, or sits on a network segment mDNS does not reach, is added by address
  through the *hosts* params.
- **BOS** (Braiins OS) miners are polled across three endpoints: `/miner/stats` (hashrate and power),
  `/miner/hw/hashboards` (the hottest chip temperature and the chip type/count), and `/miner/details` (uptime, platform,
  nominal hashrate, and miner model). A `401`/`403` triggers one re-authentication per device per pass.
- **uBOS** devices expose a single `/info` endpoint carrying hashrate, power, temperature, and uptime. uBOS advertises
  no platform identifier and no nominal hashrate, so its product name doubles as the model grouping and its nameplate
  hashrate comes from the built-in model catalog.
- **AxeOS** miners — Bitaxe and NerdQAxe++ running ESP-Miner — expose a single `/info` endpoint carrying hashrate,
  power, temperature, uptime, and nominal hashrate. The model is resolved from `deviceModel` (falling back to the board
  version), with chip type and count from `ASICModel`/`asicCount`; discovery TXT records also seed a provisional model
  hint before the first poll.

## Parameters

All parameters are manifest-driven widget settings, configurable from the web UI.

| Key               | Name              | Type    | Default    | Purpose                                                                    |
| ----------------- | ----------------- | ------- | ---------- | -------------------------------------------------------------------------- |
| `fleet_name`      | Fleet name        | string  | `My Fleet` | Heading shown above the fleet overview.                                    |
| `device_names`    | Device names      | string  | `{}`       | JSON object mapping a device mDNS name or manual host to a friendly label. |
| `bos_password`    | BOS password      | string  | `root`     | Root password used to log into every BOS miner on the network.             |
| `ubos_username`   | uBOS username     | string  | `root`     | User name for HTTP Basic auth against every uBOS device.                   |
| `ubos_password`   | uBOS password     | string  | `root`     | Password for HTTP Basic auth against every uBOS device.                    |
| `model_whitelist` | Shown models      | string  | `[]`       | JSON array of model-name fragments to show; empty shows all.               |
| `model_blacklist` | Hidden models     | string  | `[]`       | JSON array of model-name fragments to hide.                                |
| `bos_enabled`     | Show BOS miners   | boolean | `true`     | Include BOS miners in the view and keep polling them.                      |
| `ubos_enabled`    | Show uBOS miners  | boolean | `true`     | Include uBOS devices in the view and keep polling them.                    |
| `axeos_enabled`   | Show AxeOS miners | boolean | `true`     | Include AxeOS miners in the view and keep polling them.                    |
| `bos_hosts`       | BOS hosts         | string  | `[]`       | JSON array of extra BOS hosts to poll beyond mDNS; `host` or `host:port`.  |
| `ubos_hosts`      | uBOS hosts        | string  | `[]`       | JSON array of extra uBOS hosts to poll beyond mDNS.                        |
| `axeos_hosts`     | AxeOS hosts       | string  | `[]`       | JSON array of extra AxeOS hosts to poll beyond mDNS.                       |

## Constraints

- The widget targets the Deck's full **1280×480** rectangular viewport only; the manifest declares no smaller sizes.
- Font sizes are fixed for that layout.
- Every device across all families is polled on a single global round-robin, one device at a time, aiming to refresh the
  whole fleet roughly every 5 seconds — bounded by a floor on the per-device poll rate, so a large fleet's freshness
  degrades gracefully rather than the box drowning in parallel requests.
- The *BOS password*, *uBOS username*, and *uBOS password* are stored and shown as ordinary widget text (the manifest
  system has no secret-parameter type yet) and are sent unencrypted over the local network, since the miner APIs are
  HTTP-only. This is a known limitation.
- Credentials are shared fleet-wide per family; the widget cannot use different credentials for individual miners of the
  same family.
- Number formatting follows the device's localization system setting; it is not a per-widget setting.
- mDNS discovery is subject to the host runtime's browse limits; if a browse is rejected the widget logs a warning and
  that family relies on its manually configured hosts.
