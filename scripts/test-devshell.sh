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
