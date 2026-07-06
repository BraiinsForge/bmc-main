usage() {
    cat <<EOF
Usage: firmware-index-serve <proxy|local>

Serve a BMC firmware release index for upgrade testing on :8080
(override the port with BMC_FIRMWARE_INDEX_PORT).

  proxy   Mirror the internal release server
          (upstream: BMC_FIRMWARE_INDEX_UPSTREAM, default https://downloads.braiins.com.ii.zone)
  local   Host a local index and firmware files
          (root: BMC_FIRMWARE_INDEX_ROOT, default ./docs/devel/firmware)

Point the device at this machine with
  BMC_INDEX_URL="http://<lan-ip>:8080/braiins-deck"
EOF
}

# Set by the flake app wrapper; falls back to the script's own directory
# so the script also runs straight from the repo checkout.
config_dir="${FIRMWARE_INDEX_SERVE_CONFIG_DIR:-$(dirname "$0")}"

case "${1:-}" in
proxy | local) exec caddy run --config "$config_dir/Caddyfile.$1" ;;
-h | --help) usage ;;
*)
    usage >&2
    exit 1
    ;;
esac
