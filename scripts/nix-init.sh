#!/usr/bin/env bash
# Initialize a device with the Nix store tarball.
# Builds the init tarball, streams it to the device, and activates the profile.
#
# Usage: ./scripts/nix-init.sh [device-ip]
#
# Examples:
#   ./scripts/nix-init.sh 192.168.1.2
#   DEVICE_IP=192.168.1.2 ./scripts/nix-init.sh
#
# Prerequisites on device:
#   - SSH access as root
#   - /mnt/data partition available
set -euo pipefail

device="${1:-${DEVICE_IP:?Set DEVICE_IP or pass as first argument}}"
ssh_target="root@${device}"
profile_path="/nix/var/nix/gcroots/profiles/bmc"

# 1. Check if the device already has /nix or /mnt/data/nix
echo "Checking device state..."
status=$(ssh "$ssh_target" '
  nix_empty=true
  mnt_empty=true
  if [ -d /mnt/data/nix ] && [ "$(ls -A /mnt/data/nix 2>/dev/null)" ]; then
    mnt_empty=false
  fi
  if [ -d /nix ] && [ "$(ls -A /nix 2>/dev/null)" ]; then
    nix_empty=false
  fi
  if ! $mnt_empty || ! $nix_empty; then
    echo "non-empty"
  elif [ -d /mnt/data/nix ] || [ -d /nix ]; then
    echo "empty"
  else
    echo "clean"
  fi
')

case "$status" in
non-empty)
    echo "Error: /nix or /mnt/data/nix already exist and are non-empty."
    echo "To reinitialize, first remove them on the device:"
    echo "  ssh $ssh_target 'umount /nix 2>/dev/null; rm -rf /mnt/data/nix /nix'"
    exit 1
    ;;
empty | clean)
    echo "Device is ready for initialization."
    ;;
esac

# 2. Create /mnt/data/nix and bind mount to /nix
echo "Setting up /nix on device..."
ssh "$ssh_target" '
  set -e
  mkdir -p /mnt/data/nix
  mkdir -p /nix
  if ! mountpoint -q /nix; then
    mount --bind /mnt/data/nix /nix
  fi
'

# 3. Build the init tarball
echo "Building init tarball..."
tarball_path=$(nix build ".#init-tarball-armv7" --no-link --print-out-paths)
tarball_file=$(ls "$tarball_path"/*.tar.gz)
echo "Tarball: $tarball_file"

# 4+5. Stream tarball to device and extract in one pipe
echo "Streaming tarball to device and extracting..."
ssh "$ssh_target" 'tar xzf - -C /' <"$tarball_file"

# 6. Activate the profile
echo "Activating profile..."
# shellcheck disable=SC2029 # Intentional client-side expansion of profile_path
ssh "$ssh_target" "${profile_path}/1-link/core/activation/entrypoint"

echo "Done. Nix store initialized and profile activated on ${device}."
