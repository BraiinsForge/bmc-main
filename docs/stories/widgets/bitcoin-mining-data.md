# Bitcoin Mining Data Widget

The Bitcoin Mining Data widget puts current network, market, and mining-economics data on the Deck. It presents a
compact mining overview at smaller sizes and expands into charts and block-production statistics as more space becomes
available. All four rectangular widget sizes are supported.

## User stories

### See the state of Bitcoin mining at a glance

> As a miner, I want the important Bitcoin mining figures together so I can understand current network conditions at a
> glance.

- Every size shows the current Bitcoin difficulty, the estimated next adjustment and its timing, and hashprice in USD
  per PH per day.
- Medium, Large, and Fullscreen also show the previous adjustment. Medium adds the BTC-USD price and its 24-hour change.
  Large concentrates on difficulty and hashprice, while Fullscreen brings the market and network panels together with
  the mining-economics figures.
- Values use compact units that remain readable as the network grows, including automatic SI scaling for network
  hashrate.

### Read recent trends, not only snapshots

> As a miner, I want historical charts beside the current values so I can see whether conditions are moving rather than
> judging one number in isolation.

- Medium, Large, and Fullscreen show one year of Bitcoin difficulty history.
- Fullscreen adds trailing-day charts for the BTC-USD price and network hashrate, together with their 24-hour changes.
- A history containing only one value is drawn as a flat line, without inventing a change from data that does not exist.

### Understand block production and mining economics

> As a miner, I want the wider dashboard to explain block production and rewards so I can put the headline network
> figures in context.

- Fullscreen shows average fees per block, fees as a percentage of the block reward, and total mining revenue.
- It also shows the current epoch's average block time, the block height, blocks produced during the trailing 24 hours
  against the expected 144, and blocks produced this epoch against 2016.
- Epoch time is shown as zero-padded minutes and seconds, so it can be compared quickly with Bitcoin's ten-minute
  target.

### Keep useful data through connection problems

> As a user, I want the widget to distinguish loading, stale data, and an unavailable service so I know whether the
> figures on screen can still be trusted.

- Before the first response, fields use quiet placeholders rather than looking like valid zero values.
- A failed refresh keeps the last complete response on screen and marks it as stale. An already-stale server response is
  identified the same way.
- If the first request fails, the widget leaves the values unavailable and shows that Bitcoin data could not be loaded.
- When the service rate-limits a request, the widget says so and waits ten minutes before retrying that resource.

### Keep existing scenes through the upgrade

> As an existing user, I want my Bitcoin mining widget to survive the WASM upgrade so I do not have to rebuild my
> scenes.

- A scene containing the released Bitcoin mining data widget is migrated to the WASM widget while preserving its id,
  position, and size.
- The replacement needs no configuration, matching the released widget.

## Nexus API

The widget reads two Bitcoin resources from the Braiins Forge Nexus rather than contacting the upstream mining-data
providers directly.

| Resource         | Feeds                                                                    | Used by                            |
| ---------------- | ------------------------------------------------------------------------ | ---------------------------------- |
| `mining-info`    | current difficulty, prices, rewards, block production, and epoch figures | Small, Medium, Large, Fullscreen   |
| `mining-history` | difficulty, BTC price, and network hashrate histories                    | Medium, Large, and Fullscreen only |

- Each resource is requested when the widget starts and then follows the refresh lifetime advertised by Nexus.
- The current-data resource normally refreshes every 60 seconds; history normally refreshes every 10 minutes.
- Each response is accepted as a complete unit, so a malformed refresh cannot mix partial new data with the previous
  snapshot.

## Parameters

The widget has no configurable parameters.

## Constraints

- Rectangular viewports only: Small 317×238, Medium 638×238, Large 638×480, and Fullscreen 1280×480.
- Values are USD-only, matching both the released widget and the current Nexus contract.
- Alert configuration is not available until the WASM widget stack provides the required alert integration.
- Hashprice has no 24-hour change badge because neither the released widget nor Nexus provides that value.
- Charts are line charts; candlestick charts are not supported.
