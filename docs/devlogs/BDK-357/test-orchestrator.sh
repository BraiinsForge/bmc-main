#!/bin/sh
# shellcheck disable=SC3043  # 'local' is supported by ash/busybox
# Service orchestrator integration tests.
# Runs on the device. Expects TEST_*_STORE_PATH env vars.
set -eu

export PATH="/run/current-profile/bin:$PATH"

ORCH_LOG="/var/log/nix-orchestrator/nix-orchestrator.log"

fail_count=0

assert_file_exists() {
    if [ ! -f "$1" ]; then
        echo "FAIL: expected $1 to exist"
        fail_count=$((fail_count + 1))
    else
        echo "PASS: $1 exists"
    fi
}

assert_file_absent() {
    if [ -f "$1" ]; then
        echo "FAIL: expected $1 to NOT exist"
        fail_count=$((fail_count + 1))
    else
        echo "PASS: $1 absent"
    fi
}

wait_for_orchestrator() {
    local timeout=30
    local i=0
    while [ "$i" -lt "$timeout" ]; do
        if grep -q "deleting transient service" "$ORCH_LOG" 2>/dev/null; then
            return 0
        fi
        sleep 1
        i=$((i + 1))
    done
    echo "FAIL: orchestrator did not finish within ${timeout}s"
    cat "$ORCH_LOG" 2>/dev/null || true
    fail_count=$((fail_count + 1))
    return 1
}

clean_markers() {
    rm -f /tmp/init_boot /tmp/init_start
    rm -f /tmp/upgrade_reload /tmp/upgrade_failed /tmp/upgrade_init_failed
    rm -f /tmp/remove_stop
}

reset_orchestrator_log() {
    : > "$ORCH_LOG" 2>/dev/null || true
}

# ── Phase 0: Ensure clean state ──────────────────────────────────────
echo "=== Phase 0: Cleanup stale test packages ==="
bmc-nix-cli remove-packages \
    --name test-init \
    --name test-upgrade \
    --name test-remove 2>/dev/null || true
clean_markers
reset_orchestrator_log
sleep 2

# ── Phase 1: Initial install (all services new) ─────────────────────
echo "=== Phase 1: Initial install ==="
clean_markers
reset_orchestrator_log

bmc-nix-cli add-packages \
    --name test-init    --version 1.0.0 --store-path "$TEST_INIT_STORE_PATH" \
    --name test-upgrade --version 1.0.0 --store-path "$TEST_UPGRADE_V1_STORE_PATH" \
    --name test-remove  --version 1.0.0 --store-path "$TEST_REMOVE_STORE_PATH"

wait_for_orchestrator

assert_file_exists /tmp/init_boot
assert_file_exists /tmp/init_start

# ── Phase 2: Upgrade ────────────────────────────────────────────────
echo "=== Phase 2: Upgrade ==="
clean_markers
reset_orchestrator_log

bmc-nix-cli add-packages \
    --name test-upgrade --version 2.0.0 --store-path "$TEST_UPGRADE_V2_STORE_PATH"

wait_for_orchestrator

assert_file_exists  /tmp/upgrade_reload
assert_file_absent  /tmp/upgrade_failed
assert_file_absent  /tmp/upgrade_init_failed
assert_file_absent  /tmp/init_boot
assert_file_absent  /tmp/init_start

# ── Phase 3: Removal ───────────────────────────────────────────────
echo "=== Phase 3: Removal ==="
clean_markers
reset_orchestrator_log

bmc-nix-cli remove-packages \
    --name test-remove

wait_for_orchestrator

assert_file_exists  /tmp/remove_stop
assert_file_absent  /tmp/init_boot
assert_file_absent  /tmp/upgrade_reload

# ── Phase 4: Cleanup ───────────────────────────────────────────────
echo "=== Phase 4: Cleanup ==="

bmc-nix-cli remove-packages \
    --name test-init \
    --name test-upgrade

clean_markers

echo ""
echo "=== Results: $fail_count failure(s) ==="
if [ "$fail_count" -gt 0 ]; then
    exit 1
fi
echo "All tests passed."
