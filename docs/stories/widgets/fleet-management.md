# Fleet Management Widget

The fleet management widget gives an at-a-glance view of every Bitcoin miner on the local network. It discovers miners
over mDNS, polls each one for live telemetry, and rolls the readings up into a fleet total plus a per-model breakdown.
It supports three device families — BOS, uBOS, and AxeOS (Bitaxe / NerdQAxe++) — and each family is independently
discovered, authenticated, polled, and toggled, so one family failing or being disabled never blanks another's numbers.

## User stories

### See my whole fleet at a glance

> As a miner operator, I want one screen that sums up every miner on my network so I can confirm the fleet is healthy
> without opening each device.

- The widget shows the fleet total: combined hashrate (TH/s), power (W), efficiency (J/TH), and a *Mining / Online*
  count — how many miners are actively hashing versus how many are reachable.
- A miner counts as *Mining* when it reports a hashrate above 0.1 TH/s; it counts as *Online* whenever at least one of
  its telemetry endpoints answered on the last poll.
- Efficiency is total power divided by total hashrate across the group, not the mean of per-device ratios, so an idle
  but powered miner correctly drags the figure down.
- Temperature, where shown, reads as a `min/avg/max °C` range over the devices in the group; for a single-device group
  the three values collapse to one.

### Break the fleet down by model

> As an operator running mixed hardware, I want miners grouped by model so I can compare how each type is performing.

- Below the fleet total the widget lists one row per resolved model name, each with that model's hashrate, power,
  efficiency, temperature range, and mining/online counts.
- Groups are ordered by family — uBOS first, then BOS, then Bitaxe — and alphabetically by model name within a family.
- Miners whose model cannot be resolved are collected into a single *Unknown* group, always pinned last.

### Point the widget at my miners' credentials

> As an operator, I want to give the widget one set of credentials per family so it can read stats from every miner of
> that family on the network.

- *BOS password* is the `root` password used to log into every BOS miner; the username is always `root`.
- *uBOS username* and *uBOS password* are the HTTP Basic credentials used against every uBOS device.
- AxeOS miners need no credentials.
- Credentials are shared fleet-wide per family — there is one BOS password and one uBOS login for the whole network, not
  a per-device setting.

### Reach miners that mDNS does not find

> As an operator on a segmented or quiet network, I want to name miners by address so the widget polls them even when
> mDNS discovery does not surface them.

- *BOS hosts*, *uBOS hosts*, and *AxeOS hosts* are each a JSON array of extra hosts to poll, beyond mDNS discovery.
- Each entry is `host` or `host:port` (e.g. `["10.0.0.5", "miner.local:8080"]`); a bare host uses the family's default
  port (80 for BOS and AxeOS, 8080 for uBOS).
- A host found both manually and via mDNS appears twice in the fleet.
- Editing a host param reconciles the manual set live: added hosts start polling, removed hosts stop and drop their
  cached session token. Clearing a family's manual hosts requires the explicit empty array `[]`; invalid JSON or any
  other empty value leaves the manual set unchanged, so a typo cannot silently wipe the list.

### Choose which models the widget shows

> As an operator, I want to filter the fleet view down to the models I care about, or hide ones I do not.

- *Shown models* (whitelist) is a JSON array of model-name fragments; when non-empty, only models matching a fragment
  are shown. *Hidden models* (blacklist) is a JSON array of fragments to hide.
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

> As an operator, I want clear behavior when a miner is unreachable or slow to answer so the screen never shows me a
> stale value as if it were live.

- A miner contributes to the totals only while it is reachable; an unreachable miner drops out of its group and the
  total rather than freezing an old reading there.
- Failed and timed-out fetches retry on the next pass on their own; the widget keeps the last good data between passes.
- AxeOS reports `-1` for a sensor it has not read yet (notably right after boot); the widget drops these negative
  sentinels so they never pollute the fleet totals.
- Numbers use the device's configured number format for digit grouping and the decimal mark.
- Until the first miner answers, the widget shows *Searching for miners…*.

## Supported families

| Family | Display label | mDNS service             | Default port | API base      | Auth                                                   |
| ------ | ------------- | ------------------------ | ------------ | ------------- | ------------------------------------------------------ |
| BOS    | BOS           | `_bos._sub._http._tcp`   | 80           | `/api/v1`     | token login at `/auth/login` (`root` + *BOS password*) |
| uBOS   | uBOS          | `_ubos._tcp`             | 8080         | `/api`        | HTTP Basic (*uBOS username* / *uBOS password*)         |
| AxeOS  | Bitaxe        | `_axeos._sub._http._tcp` | 80           | `/api/system` | none                                                   |

- **BOS** (Braiins OS) miners are polled across three endpoints: `/miner/stats` (hashrate and power),
  `/miner/hw/hashboards` (the hottest chip temperature and the chip type/count), and `/miner/details` (uptime, platform,
  and miner model). A `401`/`403` triggers one re-authentication per device per pass.
- **uBOS** devices (e.g. the BMM network adapter) expose a single `/info` endpoint carrying hashrate, power,
  temperature, and uptime. uBOS advertises no platform identifier, so its product name doubles as the model grouping.
- **AxeOS** miners — Bitaxe and NerdQAxe++ running ESP-Miner — expose a single `/info` endpoint. The model is resolved
  from `deviceModel` (falling back to the board version), with chip type and count from `ASICModel`/`asicCount`;
  discovery TXT records also seed a provisional model hint before the first poll.

### BOS discovery quirk (bos-avahi)

The widget auto-discovers BOS miners by browsing the `_bos` mDNS service subtype, but stock BOS firmware does not
advertise it yet. For the time being this is bridged by a custom `bos-avahi` service that runs alongside the miner and
publishes the subtype; mDNS support in BOS itself is **pending submission upstream**. Until it lands, a BOS miner
without `bos-avahi` will not appear via mDNS and must be added by address through the *BOS hosts* parameter instead.

### AxeOS discovery quirk (ESP-Miner / NerdQAxe++)

The widget auto-discovers AxeOS miners by browsing the `_axeos` mDNS service subtype. Stock ESP-Miner firmware — which
both Bitaxe and NerdQAxe++ run — does not advertise that subtype yet: the support was contributed to ESP-Miner by a
third party and is **not in a released firmware build at the time of writing**. Until a release carries it, AxeOS miners
running unpatched firmware will not appear via mDNS and must be added by address through the *AxeOS hosts* parameter
instead. Once the patched firmware ships, they discover automatically like the other families.

## Parameters

All parameters are manifest-driven widget settings, configurable from the web UI.

| Key               | Name              | Type    | Default    | Purpose                                                                   |
| ----------------- | ----------------- | ------- | ---------- | ------------------------------------------------------------------------- |
| `fleet_name`      | Fleet name        | string  | `My Fleet` | Heading shown above the fleet overview.                                   |
| `bos_password`    | BOS password      | string  | `root`     | Root password used to log into every BOS miner on the network.            |
| `ubos_username`   | uBOS username     | string  | `root`     | User name for HTTP Basic auth against every uBOS device.                  |
| `ubos_password`   | uBOS password     | string  | `root`     | Password for HTTP Basic auth against every uBOS device.                   |
| `model_whitelist` | Shown models      | string  | `[]`       | JSON array of model-name fragments to show; empty shows all.              |
| `model_blacklist` | Hidden models     | string  | `[]`       | JSON array of model-name fragments to hide.                               |
| `bos_enabled`     | Show BOS miners   | boolean | `true`     | Include BOS miners in the view and keep polling them.                     |
| `ubos_enabled`    | Show uBOS miners  | boolean | `true`     | Include uBOS devices in the view and keep polling them.                   |
| `axeos_enabled`   | Show AxeOS miners | boolean | `true`     | Include AxeOS miners in the view and keep polling them.                   |
| `bos_hosts`       | BOS hosts         | string  | `[]`       | JSON array of extra BOS hosts to poll beyond mDNS; `host` or `host:port`. |
| `ubos_hosts`      | uBOS hosts        | string  | `[]`       | JSON array of extra uBOS hosts to poll beyond mDNS.                       |
| `axeos_hosts`     | AxeOS hosts       | string  | `[]`       | JSON array of extra AxeOS hosts to poll beyond mDNS.                      |

## Constraints

- The manifest declares rectangular viewports from 317×238 upward. The full per-model breakdown table needs at least a
  638×480 box; anything smaller in either dimension (BMM100 320×240, BMM101 480×320) falls back to a centered
  summary-only screen showing just the fleet total. The widest *Full* band shows every breakdown column; the narrower
  *Large* band (638 wide) keeps only the headline hashrate, the model, and the mining/online counts, since the long
  model names cannot share that row width with the numeric columns.
- Font sizes are fixed within each layout band — columns and fields are dropped on smaller viewports rather than shrunk.
- Each family is polled in round-robin passes, one device at a time, with roughly 30 seconds between passes per family.
  A device is reachable when at least one of its endpoints returned usable telemetry on the pass.
- The *BOS password*, *uBOS username*, and *uBOS password* are stored and shown as ordinary widget text because the
  manifest system has no secret-parameter type yet. This is a known limitation.
- Credentials are shared fleet-wide per family; the widget cannot use different credentials for individual miners of the
  same family.
- Number formatting follows the device's localization system setting; it is not a per-widget setting.
- mDNS discovery is subject to the host runtime's browse limits; if a family's browse is rejected the widget logs a
  warning and that family relies on its manually configured hosts.
