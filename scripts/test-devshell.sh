#!/usr/bin/env bash
# scripts/test-devshell.sh - Test devShell locally
#
# Requirements:
#   - Nix with flakes enabled
#   - User namespaces enabled (for buildFHSEnv/bwrap)
#     Ubuntu: sudo aa-complain /usr/bin/bwrap
#     Or: sudo sysctl kernel.unprivileged_userns_clone=1
#
# Note: Docker-based testing doesn't work reliably with buildFHSEnv
# due to nix daemon/store issues in containers.

set -euo pipefail

echo "=== Testing default (full) devShell ==="

nix develop <<'EOF'
set -e
echo '--- Rust build ---'
cargo build -p bmc-shared-utils
echo '  OK: cargo build succeeded'

echo '--- Frontend (tests FHS compat for node binaries) ---'
cd frontend && yarn install && just build
test -f dist/index.html && echo '  OK: dist/index.html exists'
EOF

echo "=== All tests passed ==="
