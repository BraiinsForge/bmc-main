// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

// files
pub const BOS_VERSION: &str = "/etc/bos_version";
pub const BOS_MAJOR: &str = "/etc/bos_major";
pub const BOS_MODE: &str = "/etc/bos_mode";
pub const BOS_PLATFORM: &str = "/etc/bos_platform";
pub const BOARD: &str = "/etc/board.json";
pub const FACTORY_DEFAULT: &str = "/etc/factory-default";
pub const SETUP_PENDING: &str = "/etc/setup-pending";
pub const PROC_MTD: &str = "/proc/mtd";
pub const PROC_CPUINFO: &str = "/proc/cpuinfo";
pub const ETC_HOSTS: &str = "/etc/hosts";
pub const ETC_RESOLV_CONF: &str = "/etc/resolv.conf";
pub const ETC_DNSMASQ_CONF: &str = "/etc/dnsmasq.conf";

// directories
pub const SRC_LOGS: &str = "/var/log";
pub const SRC_ETC_CONF: &str = "/etc/config";
pub const NIX_PROFILE_DIR: &str = "/nix/var/nix/gcroots/profiles/bmc";
