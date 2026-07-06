# Firmware release index test serving

Two test modes for firmware release index upgrades, served by the `firmware-index-serve` flake app (Caddy configs live
in `scripts/firmware-index-serve/`):

```bash
nix run .#firmware-index-serve -- <proxy|local>
```

The server listens on `:8080` by default; override with `BMC_FIRMWARE_INDEX_PORT`.

## 1) Proxy mode (VPN-only upstream)

Proxy the internal release server through your machine:

```bash
# Optional; defaults to https://downloads.braiins.com.ii.zone
export BMC_FIRMWARE_INDEX_UPSTREAM="https://downloads.braiins.com.ii.zone"
nix run .#firmware-index-serve -- proxy
```

On device:

```bash
export BMC_INDEX_URL="http://<your-pc-lan-ip>:8080/braiins-deck"
```

## 2) Local files mode (index + firmware files)

Serve files from `docs/devel/firmware/`:

```bash
# Optional; defaults to ./docs/devel/firmware
export BMC_FIRMWARE_INDEX_ROOT="./docs/devel/firmware"
nix run .#firmware-index-serve -- local
```

On device:

```bash
export BMC_INDEX_URL="http://<your-pc-lan-ip>:8080/braiins-deck"
```

Put local firmware files into `docs/devel/firmware/` and reference them from `docs/devel/firmware/index.v1.json`, for
example:

```json
"sysupgrade_emmc_stm32mp157c_ii3_bmc1": "http://<your-pc-lan-ip>:8080/braiins-deck/firmware_2025-06-15.tar"
```
