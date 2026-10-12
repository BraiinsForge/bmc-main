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

# Cargo's runner for wasm32 test binaries: hands each one to the wasm test runner.
# `BMC_WASM_TEST_RUNNER` names a runner already built.
# Without it the runner is built here first, in a target directory of its own,
# since the cargo that started this script holds the lock on its own.
#
# Under `--nextest-setup` (as a nextest's setup script), it builds
# the runner once per run and exports its path to every test instead.

set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
target_dir="$repo/.tmp/wasm-test-runner/target"
runner="$target_dir/debug/wasm-test"

# From the root, where no widget workspace's `[build] target` turns the build to wasm32.
# The default profile optimises, which `fast` does not, and the runner interprets every test.
# Cargo hands its runner the tested package's `CARGO_MANIFEST_*` and `CARGO_PKG_*`;
# ring's build script reruns when they change, so they would rebuild the runner per package.
build_runner() {
    (
        cd "$repo"
        unset CARGO_MANIFEST_DIR CARGO_MANIFEST_PATH "${!CARGO_PKG_@}"
        cargo build --quiet -p bmc-wasm-test-runner --target-dir "$target_dir"
    )
}

if [ "${1:-}" = "--nextest-setup" ]; then
    if [ -z "${BMC_WASM_TEST_RUNNER:-}" ]; then
        build_runner
        echo "BMC_WASM_TEST_RUNNER=$runner" >>"$NEXTEST_ENV"
    fi
    exit 0
fi

if [ -n "${BMC_WASM_TEST_RUNNER:-}" ]; then
    exec "$BMC_WASM_TEST_RUNNER" "$@"
fi

build_runner
exec "$runner" "$@"
