# BMM Mining Widget Design

## Goal

Implement BDK-499 as one production WASM widget for BMM mining displays. The widget shows BOSer-derived mining and
Bitcoin network screens through a view enum parameter rather than as separate widget catalog entries.

Primary targets are BMM100 (`320x240`) and BMM101 (`480x320`) fullscreen rectangular viewports. BMC100 rectangular
viewports should work on a best-effort basis, but BMC100 support must not compromise BMM behavior.

## Scope

Create one widget named `bmm-mining` under `widgets-wasm/`. It should be implemented with the existing WASM widget SDK
and registered in the current WASM widget build/catalog plumbing.

In scope:

- `mining` view.
- `geek` view.
- `network` view.
- `info_overload` view.
- Miner-local BOS REST data fetching.
- Public Bitcoin network/economics data fetching through `public-api.braiins.com`.
- Responsive field degradation for small viewports.

Out of scope:

- Standalone `bitcoin`, `btc_graph`, `clock`, or device network setup/status widgets.
- Secret widget parameters. `miner_password` is intentionally a regular string parameter in BDK-499.
- Direct reuse of BOSer Rust or Slint code. BOSer and `boser-assets` are the reference for field sets, hierarchy, and
  spacing decisions.

## Manifest Parameters

The widget manifest declares these params:

- `view`: required string enum, default `mining`. Values: `mining`, `geek`, `network`, `info_overload`.
- `miner_url`: required URI string, default `http://miner/api/v1`.
- `miner_password`: string parameter used for BOS REST login. The username is hardcoded as `root`.
- `currency`: required string enum, default `usd`. Values: `usd`, `eur`.

`miner_password` is stored and shown as normal text because the widget manifest system does not currently provide a
secret/password parameter type. This is a known defect and should be documented in the implementation.

## Data Sources

The widget keeps miner-local data independent from public Bitcoin data so one source can fail without blanking unrelated
fields.

### Miner REST API

The widget logs in with:

```text
POST {miner_url}/auth/login
{"username":"root","password": miner_password}
```

The login response returns `token` and `timeout_s`. Authenticated requests use:

```text
Authorization: Bearer <token>
```

The widget caches the token. If a request returns `401`, it clears the token and logs in again. Normal authenticated
requests refresh the token validity on the miner side.

Use these request/response endpoints:

- `GET /miner/details` for BOSMiner uptime, sticker/nominal hashrate, miner status, and identity/details where
  available.
- `GET /miner/stats` for real hashrate, power stats, efficiency, found blocks, and related aggregate mining stats.
- `GET /miner/hw/hashboards` for board/chip temperatures and per-hashboard stats.
- `GET /cooling/state` for fan RPM/target ratio and highest temperature.
- `GET /network/` for the miner's active IPv4 address shown by `mining` and `geek`.

Avoid streaming endpoints, especially `/miner/status`, because the current WASM fetch API is request/response oriented.

Miner-local data should refresh roughly every 5 seconds. Network/API failures keep the last good data for a short grace
period; before first success, fields render as unavailable.

### Public Bitcoin API

Use the same public API family used by BOSer and the existing blockheight widget:

- `https://public-api.braiins.com/v1/price-stats?currency={currency}` for BTC price and 24h price change.
- `https://public-api.braiins.com/v2/blocks?limit=1&currency={currency}` for block height.
- `https://public-api.braiins.com/v1/difficulty-stats?currency={currency}` for previous/estimated difficulty adjustment
  and epoch progress.
- `https://public-api.braiins.com/v2/hashrate-stats?currency={currency}` for network hashrate, fees,
  hashprice/hashvalue, and total mining revenue where needed.

Public data should refresh roughly every 60 seconds. Failed public fetches must not clear miner-local fields.

The BOSer `info_overload` screen includes a small BTC price graph. The widget may fetch price-history data for that
graph, but the graph is the first element to hide on BMM100 if it does not fit cleanly.

## View Field Sets

Field sets are validated against `boser-assets/display/ui/screens/*.slint`.

### `mining`

Show all fields on BMM100 and larger:

- Current Hashrate, unit `TH/s`.
- Temperature, unit `°C`.
- Power Consumption, unit `W`.
- MCR, unit `%`.
- Fan Speed, unit `%`.
- IP Address.

### `geek`

Show all fields on BMM100 and larger:

- Current Hashrate, unit `TH/s`.
- Temperature, unit `°C`.
- Power Consumption, unit `W`.
- Miner Uptime.
- IP Address.
- BTC Price.

### `network`

BMM101 and BMC100 target fields:

- Network HR, unit `EH/s`.
- Diff. Adjustment, unit `%`.
- Est. Diff. Adjustment, unit `%`.
- Epoch Progress, unit `%`.
- Fees (144 Blocks), unit `BTC`, with fee percent as extra info.
- Block Height.
- Hashprice, unit `TH/Day`.
- BTC Price.

BMM100/small fallback hides these first, matching BOSer's `extra_info_visibility: false` behavior:

- Est. Diff. Adjustment.
- Epoch Progress.
- Fee percent extra info.

### `info_overload`

BMM101 and BMC100 target fields:

- Bitcoin 24h change.
- Small BTC price graph.
- BTC Price.
- Hashrate, unit `TH/s`.
- Power Consump., unit `W`.
- Block Height.
- Est. Diff. Adjust., unit `%`.
- Prev. Diff. Adjust., unit `%`.
- Epoch Progress, unit `%`.
- Miner Uptime.
- Fees (144 Blocks), as fee percent.
- Hashvalue, unit `SAT/TH/Day`.

BMM100/small fallback keeps the enum value available and hides less important information when it cannot fit. Priority:

1. Keep BTC 24h change, BTC Price, Hashrate, Power Consump., Block Height, and Miner Uptime.
2. Hide the small BTC price graph first.
3. Then hide Fees percent and Hashvalue.
4. Then hide Est. Diff. Adjust., Prev. Diff. Adjust., and Epoch Progress if the panel is still cramped.

## Rendering Behavior

Use `widget_viewport()` as the source of truth for geometry. Do not hardcode a platform from the viewport size, but it
is fine to choose responsive layout bands from the actual width/height.

Visual direction:

- Preserve BOSer content structure and hierarchy rather than the spacing in the brainstorming browser mockup.
- `mining` and `geek` use vertical `TextLine`-style rows with label left and value/unit right.
- `network` uses a two-column `TextBlock` grid where space allows and drops secondary fields on BMM100.
- `info_overload` uses a top BTC band plus dense field grid on BMM101/BMC100, and a reduced field set on BMM100.
- Prefer hiding secondary fields over unreadable text, overlap, or viewport- width-scaled font sizes.
- Keep stable dimensions and padding derived from BOSer Slint theme values where practical.

Formatting:

- Use BOSer labels and units.
- Use `N/A` for unavailable miner values.
- Use `--` where existing public-data widgets use it for not-yet-loaded numeric public data.
- Apply the device `number_format` system setting where practical; otherwise match existing production widget formatting
  and avoid false precision.
- Temperature is displayed in Celsius, matching BOSer BMM screens.

## Error Handling

Normal network/API failures are not fatal:

- Before first successful fetch, show placeholders.
- If miner data fails, public Bitcoin/network fields remain populated.
- If public API data fails, miner fields remain populated.
- If `miner_password` is empty or login fails, miner fields remain unavailable and public fields still render where the
  selected view uses them.
- Log warnings for failed fetches and malformed payloads.
- Do not panic on expected API/network/auth failures.

Panic only for internal invariants, using project convention `expect("BUG: ...")` when needed.

## Tests And Verification

Add focused unit tests for:

- Miner REST JSON parsing and field extraction.
- Public API JSON parsing and field extraction.
- Formatting of hashrate, temperature, power, MCR, uptime, difficulty adjustments, fees, BTC price, block height,
  hashprice, and hashvalue.
- Auth state transitions: no token -> login -> authenticated fetch; `401` -> clear token -> relogin.
- View degradation decisions for BMM100 versus BMM101/BMC100-sized viewports.

Add capture/regression fixtures for at least BMM100 and BMM101 fullscreen viewports. Prefer one fixture per view if
practical. If the full matrix becomes too large, prioritize `mining` and `info_overload` first, because they cover the
main miner surface and the densest responsive layout.

Visual fixtures must record fetch responses and must not depend on live APIs.

Verification should include:

- Targeted widget crate tests.
- `just wasm::verify <widget>` or the equivalent WASM regression command.
- Build/catalog checks needed for a new production WASM widget.

## Open Risks

The WASM fetch API supports methods, headers, and bodies, so REST login should work. The implementation still needs to
verify that the runtime/testbed capture path handles POST fixtures and multiple delayed fetches cleanly. If that turns
out to be fragile, serialize the widget's fetch cycle and keep fixtures deterministic.

BMC100 support is best effort. It should not delay or distort BMM100/BMM101 behavior.
