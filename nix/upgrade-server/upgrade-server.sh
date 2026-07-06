#!/usr/bin/env bash
# Serve the local /nix/store as a signed binary cache plus a package
# index a Deck device can upgrade from, and print the register-server
# command to run on the device.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: upgrade-server --package NAME=VERSION=STORE_PATH... [options]

Serve the local /nix/store as a signed binary cache and publish a
nix-package-index.v1.json plus a servers.json fragment for a Deck
device.

Options:
  --package NAME=VERSION=STORE_PATH
                     Index entry (repeatable). Overrides a same-name
                     entry from --base-index.
  --base-index FILE  Existing nix-package-index.v1.json used as the
                     baseline. The served index must contain every
                     installed system package (at minimum core), or the
                     device reports packages-unavailable.
  --port N           Binary cache port (default 8080).
  --index-port N     Package index port (default: --port + 1).
  --host ADDR        Address advertised in the printed URLs (default:
                     best-effort autodetection).
  --key-dir DIR      Signing keypair location, generated when missing
                     (default: $XDG_STATE_HOME/bmc-upgrade-server).
  -h, --help         Show this help.
EOF
}

die() {
    echo "error: $*" >&2
    exit 1
}

port=8080
index_port=""
host=""
key_dir="${XDG_STATE_HOME:-$HOME/.local/state}/bmc-upgrade-server"
base_index=""
packages=()

while [ $# -gt 0 ]; do
    arg="$1"
    case "$arg" in
    -h | --help)
        usage
        exit 0
        ;;
    --package | --base-index | --port | --index-port | --host | --key-dir)
        [ $# -ge 2 ] || die "missing value for $arg"
        value="$2"
        shift 2
        case "$arg" in
        --package) packages+=("$value") ;;
        --base-index) base_index="$value" ;;
        --port) port="$value" ;;
        --index-port) index_port="$value" ;;
        --host) host="$value" ;;
        --key-dir) key_dir="$value" ;;
        esac
        ;;
    *)
        usage >&2
        die "unknown argument: $arg"
        ;;
    esac
done

if [ "${#packages[@]}" -eq 0 ]; then
    usage >&2
    die "at least one --package NAME=VERSION=STORE_PATH is required"
fi
[ -n "$index_port" ] || index_port=$((port + 1))

if [ -z "$host" ]; then
    host=$(ip route get 1.1.1.1 2>/dev/null | sed -n 's/.*src \([^ ]*\).*/\1/p' | head -n 1 || true)
    [ -n "$host" ] || host=127.0.0.1
fi

secret_key="$key_dir/secret"
public_key="$key_dir/public"
mkdir -p "$key_dir"
chmod 700 "$key_dir"
if [ ! -f "$secret_key" ]; then
    nix-store --generate-binary-cache-key dev-upgrade "$secret_key" "$public_key"
fi
chmod 600 "$secret_key"
cache_public_key=$(<"$public_key")

work_dir=$(mktemp -d)
server_pids=()
cleanup() {
    [ "${#server_pids[@]}" -eq 0 ] || kill "${server_pids[@]}" 2>/dev/null || true
    rm -rf "$work_dir"
}
trap cleanup EXIT

index_file="$work_dir/nix-package-index.v1.json"
if [ -n "$base_index" ]; then
    jq -e '.version == 1' "$base_index" >/dev/null \
        || die "$base_index is not a version-1 package index"
    cp "$base_index" "$index_file"
else
    jq -n '{version: 1, provenance: null, indexes: [], caches: [], packages: []}' >"$index_file"
fi

for spec in "${packages[@]}"; do
    IFS='=' read -r name version store_path <<<"$spec"
    if [ -z "$name" ] || [ -z "$version" ] || [ -z "$store_path" ]; then
        die "invalid --package '$spec', expected NAME=VERSION=STORE_PATH"
    fi
    [ -e "$store_path" ] || die "store path does not exist: $store_path"
    jq --arg name "$name" --arg version "$version" --arg store_path "$store_path" \
        '.packages = [.packages[] | select(.name != $name)] + [{
       name: $name,
       version: $version,
       store_path: $store_path,
       category: null,
       description: null,
       upgrade_strategy: null,
       install_strategy: null
     }]' "$index_file" >"$index_file.tmp"
    mv "$index_file.tmp" "$index_file"
done

base_url="http://$host:$index_port"
cache_url="http://$host:$port"

# Materialize widget assets referenced by store path in each package's
# metadata.assets map and rewrite those paths to URLs this static server
# hosts, so the frontend can fetch icons (and future previews) without
# realizing a package. The store is world-readable; the symlink lives in
# the ephemeral work_dir and is removed with it. store_path and other
# metadata (bmc_version, widget picker fields) are left untouched.
if jq -e '[.packages[].metadata.assets? // empty] | length > 0' "$index_file" >/dev/null; then
    ln -s /nix/store "$work_dir/store"
    jq --arg base "$base_url" '
        .packages |= map(
            if .metadata.assets? then
                .metadata.assets |= walk(
                    if type == "string" and startswith("/nix/store/")
                    then $base + "/store/" + ltrimstr("/nix/store/")
                    else . end
                )
            else . end
        )
    ' "$index_file" >"$index_file.tmp"
    mv "$index_file.tmp" "$index_file"
fi

jq -n --arg base_url "$base_url" --arg key "$cache_public_key" \
    '{id: "dev-upgrade", type: "http", base_url: $base_url, known_public_key: $key, priority: 50, enabled: true}' \
    >"$work_dir/servers.json"

# Compression off: narinfo FileSize is then the exact wire size, so the
# device's download totals and estimates are accurate.
cat >"$work_dir/harmonia.toml" <<EOF
bind = "0.0.0.0:$port"
workers = 4
sign_key_paths = [ "$secret_key" ]
enable_compression = false
EOF
CONFIG_FILE="$work_dir/harmonia.toml" harmonia-cache &
cache_pid=$!
server_pids+=("$cache_pid")
python3 -m http.server --bind 0.0.0.0 --directory "$work_dir" "$index_port" &
index_pid=$!
server_pids+=("$index_pid")

index_url="http://127.0.0.1:$index_port/nix-package-index.v1.json"
cache_info_url="http://127.0.0.1:$port/nix-cache-info"
want_hash=$(sha256sum "$index_file" | cut -d' ' -f1)
deadline=$((SECONDS + 15))
while true; do
    kill -0 "$cache_pid" 2>/dev/null \
        || die "binary cache exited before serving; is port $port already in use?"
    kill -0 "$index_pid" 2>/dev/null \
        || die "package index exited before serving; is port $index_port already in use?"
    got_hash=$(curl -fsS "$index_url" 2>/dev/null | sha256sum | cut -d' ' -f1) || true
    if [ "$got_hash" = "$want_hash" ] && curl -fsS "$cache_info_url" >/dev/null 2>&1; then
        break
    fi
    [ "$SECONDS" -lt "$deadline" ] \
        || die "servers did not come up within 15s; a stale or foreign process may hold port $port or $index_port"
    sleep 0.2
done

cat <<EOF

binary cache:     $cache_url
package index:    $base_url/nix-package-index.v1.json
cache public key: $cache_public_key

register on the device (the index is not signed, so the index key
mirrors the cache key):

  bmc-nix-cli register-server \\
    --id dev-upgrade \\
    --base-url $base_url \\
    --index-public-key '$cache_public_key' \\
    --cache-url $cache_url \\
    --cache-public-key '$cache_public_key'

EOF

wait
