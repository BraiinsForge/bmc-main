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

# First-boot setup for bmc-virt VM environment.
# Runs once, then deleted by OpenWrt's init.

set -e

# Directories expected by bmc-openwrt
mkdir -p /www/bmc/assets /www/bmc/var /mnt/data/www-bmc

# Root password
echo 'root:root' | chpasswd 2>/dev/null || true

# Disable dropbear binding restriction so SSH works on all interfaces
uci set dropbear.@dropbear[0].Interface=""
uci commit dropbear

# Internet access — QEMU user-mode gateway is at .2
uci set network.lan.gateway='192.168.1.2'
uci set network.lan.dns='8.8.8.8'
uci commit network

# Direct DNS fallback — dnsmasq may not forward on first boot
echo "nameserver 8.8.8.8" >>/etc/resolv.conf

# DRM device node creation is handled by a-bmc-virt-setup init script.

# Fake backlight sysfs (read/write, no real hardware)
mkdir -p /tmp/fake-backlight/display-bl
echo 255 >/tmp/fake-backlight/display-bl/max_brightness
echo 0 >/tmp/fake-backlight/display-bl/bl_power
echo 128 >/tmp/fake-backlight/display-bl/brightness
echo 128 >/tmp/fake-backlight/display-bl/actual_brightness
mount --bind /tmp/fake-backlight /sys/class/backlight

# Factory-default and setup-pending flags (real hardware has these)
touch /etc/factory-default
touch /etc/setup-pending

# --- WiFi emulation network plumbing (BDK-397) ---
# One-time UCI setup for the wifinet subnet, wifi_sta interface, DHCP, and firewall.
# Radio1 wireless config is handled by the deploy script (UCI + wifi reload after app start).

# Network interface for radio1's AP (separate subnet, DHCP server for STA clients)
uci set network.wifinet=interface
uci set network.wifinet.proto='static'
uci set network.wifinet.ipaddr='10.99.0.1'
uci set network.wifinet.netmask='255.255.255.0'

# Network interface for when radio0 switches to STA mode (DHCP client).
# The app sets network='wifi_sta' when configuring STA mode (bmc-net/bmc-net-types/src/wifi.rs).
uci set network.wifi_sta=interface
uci set network.wifi_sta.proto='dhcp'

uci commit network

# DHCP pool on wifinet so radio0-as-STA gets an address
uci set dhcp.wifinet=dhcp
uci set dhcp.wifinet.interface='wifinet'
uci set dhcp.wifinet.start='100'
uci set dhcp.wifinet.limit='50'
uci set dhcp.wifinet.leasetime='1h'
# Don't advertise a default gateway from wifinet DHCP — the wifi_sta
# interface should not override the system default route (which goes
# via br-lan to the QEMU gateway). Only provide DNS.
uci add_list dhcp.wifinet.dhcp_option='3'
uci add_list dhcp.wifinet.dhcp_option='6,192.168.1.3'
uci commit dhcp

# Firewall: allow wifinet traffic and NAT it through lan (eth0 -> QEMU gateway)
uci add firewall zone
uci set firewall.@zone[-1].name='wifinet'
uci set firewall.@zone[-1].input='ACCEPT'
uci set firewall.@zone[-1].output='ACCEPT'
uci set firewall.@zone[-1].forward='ACCEPT'
uci add_list firewall.@zone[-1].network='wifinet'

uci add firewall forwarding
uci set firewall.@forwarding[-1].src='wifinet'
uci set firewall.@forwarding[-1].dest='lan'

# Enable masquerade on lan zone so wifinet traffic gets NATted out through eth0
uci set firewall.@zone[0].masq='1'
uci commit firewall

exit 0
