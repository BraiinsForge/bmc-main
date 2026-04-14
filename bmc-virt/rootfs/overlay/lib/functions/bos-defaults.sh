#!/bin/sh
# Stub for bmc-virt — real implementation lives in bos-main OpenWrt fork.
# Provides minimal definitions so bmc-openwrt doesn't fail on missing functions.

export BOS_MODE="nand"

is_factory_default() {
    [ -f /etc/factory-default ]
}

is_setup_pending() {
    [ -f /etc/setup-pending ]
}

is_wifi_reconfig() {
    [ -f /etc/wifi-reconfig ]
}

unset_factory_default() {
    rm -f /etc/factory-default
}

unset_setup_pending() {
    rm -f /etc/setup-pending
}

set_wifi_reconfig() {
    touch /etc/wifi-reconfig
}

unset_wifi_reconfig() {
    rm -f /etc/wifi-reconfig
}
