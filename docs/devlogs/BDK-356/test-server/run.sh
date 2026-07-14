#!/usr/bin/env bash
# Test server for bmc-nix-init: builds the init tarball, starts a
# local Caddy file server, and deploys servers.json to the device.
#
# Usage:
#   ./run.sh <HOST_IP> <DEVICE_IP>
# Prerequisites:
#   - nix (for building the tarball and running caddy)
#   - ssh access to the device as root

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

HOST_IP="${1:?Usage: $0 <HOST_IP> <DEVICE_IP>}"
DEVICE_IP="${2:?Usage: $0 <HOST_IP> <DEVICE_IP>}"

SERVE_DIR="$SCRIPT_DIR/.serve"
mkdir -p "$SERVE_DIR"

# Step 1: Build the init tarball
echo "==> Building init tarball..."
TARBALL_PATH=$(nix build "$REPO_DIR#init-tarball-armv7" --no-link --print-out-paths)
cp "$TARBALL_PATH"/nix-*.tar.gz "$SERVE_DIR/init-tarball.tar.gz"
echo "    Tarball: $SERVE_DIR/init-tarball.tar.gz ($(du -h "$SERVE_DIR/init-tarball.tar.gz" | cut -f1))"

# Step 2: Generate nix-factory.v1.json and servers.json with actual IP
echo "==> Generating config files for $HOST_IP..."
sed "s/\${HOST_IP}/$HOST_IP/g" "$SCRIPT_DIR/nix-factory.v1.json" >"$SERVE_DIR/nix-factory.v1.json"
sed "s/\${HOST_IP}/$HOST_IP/g" "$SCRIPT_DIR/servers.json" >"/tmp/bmc-nix-init-servers.json"

# Step 3: Deploy servers.json to device
echo "==> Deploying servers.json to $DEVICE_IP..."
ssh "root@$DEVICE_IP" "mkdir -p /etc/nix-upgrade"
scp "/tmp/bmc-nix-init-servers.json" "root@$DEVICE_IP:/etc/nix-upgrade/servers.json"

# Step 4: Build and deploy the init binary
echo "==> Building ARM init binary..."
BINARY_PATH=$(nix build "$REPO_DIR#bmc-nix-init-armv7-release" --no-link --print-out-paths)
echo "==> Deploying binary to $DEVICE_IP..."
scp "$BINARY_PATH/bin/bmc-nix-init" "root@$DEVICE_IP:/tmp/bmc-nix-init"

# Step 5: Start Caddy
echo "==> Starting Caddy on :9080..."
export SERVE_DIR
nix-shell -p caddy --run "caddy start --config '$SCRIPT_DIR/Caddyfile'" 2>&1 | grep -E "level|error|started|running"

echo ""
echo "==> Ready! Run on device:"
echo "    ssh root@$DEVICE_IP '/tmp/bmc-nix-init --servers-config /etc/nix-upgrade/servers.json'"
echo ""
echo "    To stop Caddy: nix-shell -p caddy --run 'caddy stop'"
