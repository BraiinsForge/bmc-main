# BDK-506: Fleet management widget

Fetched from Jira with `jira issue view BDK-506` on 2026-06-03.

URL: https://braiins.atlassian.net/browse/BDK-506

Status: In Progress
Type: Story
Priority: High
Assignee: Frantisek Bohacek

## Summary

Add a fleet-management widget that gives an at-a-glance view of the Bitcoin
miners on the local network, summarising fleet health, hashrate, power, and
efficiency directly on the Deck.

## Description

As a home/small-fleet miner, I want my Deck to show the state of all my
miners on the local network so that I can tell at a glance whether the fleet
is healthy without opening a separate tool.

The widget periodically discovers miners on the local network and polls each
one for basic telemetry, then presents an aggregate fleet summary and a
per-model breakdown. The emphasis is on fleet confidence, not on per-device
drilldown.

Supported device families in the initial scope:

- BOS+ - BMM, BFM
- uBOS - HashNode
- AxeOS - NerdQaxe++
- extensible to further families later

## Per-Device Data

For each discovered device the widget reads:

- power in W
- current hashrate in TH/s
- nominal nameplate hashrate in TH/s
- uptime in hours
- temperature in deg C

## Derived Metrics

There is no per-device view. Data is presented only at the fleet
all-devices level and per-model level.

All aggregates are computed from the latest telemetry poll only. Previous
values are never carried forward, and a device with missing readings is
excluded from the affected aggregate rather than contributing stale data.

- okay / not-okay: a device is not okay when its current hashrate is below
  20 percent of its nominal hashrate, otherwise okay. Classification uses
  the latest telemetry poll. Only aggregate counts are shown.
- efficiency in J/TH: computed as total power in W divided by total
  hashrate in TH/s over the devices in the group, not as an average of
  per-device efficiencies. Devices reporting zero hashrate are excluded
  from both numerator and denominator.
- temperature: minimum, maximum, and average in deg C across devices in the
  group that reported a fresh temperature.

## Displayed Views

1. All-devices overview: total hashrate, total power, fleet efficiency,
   min/max/avg temperature, online count, okay count, and not-okay count.
2. Per-model breakdown: for each model, hashrate, power, efficiency,
   min/max/avg temperature, online count, okay count, and not-okay count.

Both views show when the data was last refreshed.

## Size Behavior

- Full variant shows both views together.
- Large variant shows only one of the two views at a time.
- Small and medium variants are not expected to be supported.
- The final decision on supported sizes and unsupported-size behavior must
  be recorded as part of the story.

## Discovery and Refresh

- Devices are discovered via mDNS on the local network.
- Discovery runs periodically on a slower cadence than telemetry polling.
- Telemetry for known devices is polled roughly every 30 seconds.
- Last refreshed reflects the most recent telemetry cycle that successfully
  produced the displayed aggregates, not merely the last poll attempt.

## Device Presence

- The widget only knows about devices it has discovered online.
- There is no fixed expected fleet.
- An unreachable device is omitted and is not included in any count or
  aggregate.
- There is no offline or stale state.

## Empty State

When no miners have been discovered, the widget shows an explicit empty
state rather than blank or zeroed totals.

## Persistence

The widget does not persist data. Discovered devices and telemetry live only
in memory for the current session. After restart, the fleet is rebuilt from
scratch through discovery and polling.

## Acceptance Criteria

- [ ] Discover miners on the local network via mDNS across BOS+ BMM/BFM,
  uBOS HashNode, and AxeOS NerdQaxe++.
- [ ] Retrieve power, current hashrate, nominal hashrate, uptime, and
  temperature for each device.
- [ ] Classify a device as not okay when current hashrate is below
  20 percent of nominal hashrate on the latest telemetry poll; surface only
  aggregate counts.
- [ ] Show all-devices overview with total hashrate, total power, fleet
  efficiency, min/max/avg temperature, online count, okay count, and
  not-okay count.
- [ ] Show per-model breakdown with hashrate, power, efficiency,
  min/max/avg temperature, online count, okay count, and not-okay count for
  each model.
- [ ] Report efficiency and min/max/avg temperature per model and
  fleet-wide, not per device.
- [ ] Compute efficiency as total power divided by total hashrate over the
  group, not as an average of per-device efficiencies.
- [ ] Exclude devices reporting zero hashrate from efficiency numerator and
  denominator.
- [ ] Use only latest telemetry for all aggregates.
- [ ] Exclude missing readings rather than substituting previous values.
- [ ] Track, count, and aggregate only currently reachable devices.
- [ ] Show an explicit empty state when no miners are discovered.
- [ ] Persist no data.
- [ ] Show last refreshed for both views, reflecting the last successful
  telemetry cycle.
- [ ] Refresh known-device telemetry roughly every 30 seconds.
- [ ] Run discovery periodically on a slower cadence.
- [ ] Record the small/medium support decision.
- [ ] Full variant shows both views together.
- [ ] Large variant shows one view at a time.

## Out of Scope

- Per-device view, drilldown, or controls.
- Historical charts or retained time-series data.
- Pool-side or effective hashrate.

## Current Implementation Notes

- BOS Avahi advertisement prerequisite is implemented in commit
  `24945416bb2b`.
- Next design topic: widget skeleton plus generic family/model support for
  BOS, uBOS, and Bitaxe/AxeOS-style devices.
