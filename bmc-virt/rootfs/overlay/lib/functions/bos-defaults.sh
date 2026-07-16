#!/bin/sh
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
