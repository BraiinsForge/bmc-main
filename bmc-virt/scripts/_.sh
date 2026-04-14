# shellcheck shell=bash
# Shared helpers for bmc-virt scripts.
# Source this: . "$(dirname "$0")/_.sh"
#
# Scripts using this should have a nix shebang that provides:
#   openssh, sshpass, coreutils

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VIRT_DIR="$(dirname "$SCRIPT_DIR")"

export DATADIR="${BMC_VIRT_DATA:-$VIRT_DIR/vm-data}"
export LOGDIR="$DATADIR/logs"
export SSH_OPTS=(-F /dev/null -o StrictHostKeyChecking=no -o UserKnownHostsFile="$DATADIR/known_hosts" -o WarnWeakCrypto=no -p 2222)

ssh_vm() {
    sshpass -p root ssh "${SSH_OPTS[@]}" root@localhost "$@"
}

scp_vm() {
    sshpass -p root scp -F /dev/null -o StrictHostKeyChecking=no -o UserKnownHostsFile="$DATADIR/known_hosts" -o WarnWeakCrypto=no -P 2222 -O "$@"
}
