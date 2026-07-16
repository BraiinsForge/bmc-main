#!/usr/bin/env bash
# Copyright (C) 2026  Braiins Forge s.r.o.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
# to grant any party a license to this program, or any part thereof,
# under any terms, and such a grant shall be considered distinct from
# the grant above.

# Ensure a linux-builder VM with x86_64 binfmt is running (macOS only).
# Uses our custom NixOS builder from the flake, which has
# boot.binfmt.emulatedSystems = ["x86_64-linux"] for the ImageBuilder.
#
# All builder state (keys, disk image, logs) lives in a single data dir
# so nothing pollutes the project tree.

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
    exit 0
fi

# All builder state goes here — persistent across reboots.
DATA_DIR="$HOME/.local/share/bmc-virt/builder"
mkdir -p "$DATA_DIR/keys"

USER_KEY="$DATA_DIR/keys/builder_ed25519"
LOG="$DATA_DIR/builder.log"

builder_ready() {
    nc -z localhost 31022 2>/dev/null || return 1
    [[ -f $USER_KEY ]] && ssh -i "$USER_KEY" -o StrictHostKeyChecking=no -o ConnectTimeout=2 -o BatchMode=yes linux-builder true 2>/dev/null
}

if builder_ready; then
    echo "Linux builder already running."
    exit 0
fi

# Kill any leftover QEMU holding the builder port.
lsof -ti :31022 2>/dev/null | xargs kill 2>/dev/null || true
sleep 0.5

echo "Linux builder not running, starting..."

# Generate a persistent user keypair (the system one at /etc/nix/ is root-only).
if [[ ! -f $USER_KEY ]]; then
    ssh-keygen -q -t ed25519 -N "" -C "builder@localhost" -f "$USER_KEY"
fi

# Build/fetch our custom linux-builder with binfmt support.
WORKSPACE=$(cd "$(dirname "$0")/../.." && pwd)
BUILDER=$(nix build -L "path:$WORKSPACE/bmc-virt#linuxBuilder" --no-link --print-out-paths 2>&1 | tail -n 1)
if [[ ! -x "$BUILDER/bin/create-builder" ]]; then
    echo "ERROR: Failed to build linuxBuilder:" >&2
    echo "$BUILDER" >&2
    exit 1
fi

# Prepare VM keys: combine our pub key + system pub key so both we and
# nix-daemon can SSH in. Use a separate dir so the original keypair stays intact.
VM_KEYS=$(mktemp -d "$DATA_DIR/vm-keys.XXXXXX")
cp "$USER_KEY" "$VM_KEYS/builder_ed25519"
cat "$USER_KEY.pub" /etc/nix/builder_ed25519.pub >"$VM_KEYS/builder_ed25519.pub"

# Extract run-builder path from create-builder (avoid add-keys which needs sudo).
RUN_BUILDER=$(grep -o '/nix/store/[^ ]*/bin/run-builder' "$BUILDER/bin/create-builder")
KEYS="$VM_KEYS" bash -c "cd '$DATA_DIR' && exec '$RUN_BUILDER'" >"$LOG" 2>&1 &
BUILDER_PID=$!

# Tail the log so the user sees boot progress (strip terminal escape sequences).
tail -f "$LOG" 2>/dev/null | sed 's/\x1b\[[0-9;]*[a-zA-Z]//g' &
TAIL_PID=$!
trap 'kill $TAIL_PID 2>/dev/null' EXIT

echo "Waiting for builder VM to boot..."

for _i in $(seq 1 120); do
    if builder_ready; then
        echo "Linux builder ready (with x86_64 binfmt)."
        exit 0
    fi
    if ! kill -0 "$BUILDER_PID" 2>/dev/null; then
        echo "ERROR: Builder process exited." >&2
        tail -5 "$LOG" >&2
        exit 1
    fi
    sleep 1
done

echo "ERROR: Builder did not become ready in 120s." >&2
exit 1
