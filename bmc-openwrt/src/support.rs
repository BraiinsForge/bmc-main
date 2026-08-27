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

//! The OpenWrt board's support-archive recipe: the BMC-specific constants,
//! command/path/host sets, credential filters and extensions, assembled into
//! the [`SupportConfig`] the manager collects through.

mod consts;

use bmc_support::{SupportConfig, SupportExtension, SupportFilter};
use bmc_support_openwrt::{
    BMC_CONFIG_DIR, BMC_CONFIG_LEGACY, BmcConfigCensor, LogReadExtension, NixProfileExtension,
    SecretsExclusion, UciWirelessCensor,
};
use consts::{
    BOARD, BOS_MAJOR, BOS_MODE, BOS_PLATFORM, BOS_VERSION, ETC_DNSMASQ_CONF, ETC_HOSTS,
    ETC_RESOLV_CONF, FACTORY_DEFAULT, NIX_PROFILE_DIR, PROC_CPUINFO, PROC_MTD, SETUP_PENDING,
    SRC_ETC_CONF, SRC_LOGS,
};
use std::sync::LazyLock;

/// Commands whose stdout is captured into the support archive.
const COMMANDS: &[&[&str]] = &[
    &["dmesg"],
    &["fw_printenv"],
    &["env"],
    &["ifconfig", "-a"],
    &["ip", "addr"],
    &["ip", "route"],
    &["ps", "aux"],
    &["df"],
    &["ls", "-l", "/tmp"],
    &["killall", "-SIGUSR1", "dnsmasq"],
];

/// All contents of these paths are included in the support archive.
const DEVICE_FS_PATHS: &[&str] = &[
    // files
    BOS_VERSION,
    BOS_MAJOR,
    BOS_MODE,
    BOS_PLATFORM,
    ETC_HOSTS,
    ETC_RESOLV_CONF,
    ETC_DNSMASQ_CONF,
    BOARD,
    // Pre-migration config, kept on disk for downgrade safety. The
    // current config and its timestamped backups come in via the
    // BMC_CONFIG_DIR directory below.
    BMC_CONFIG_LEGACY,
    FACTORY_DEFAULT,
    SETUP_PENDING,
    // directories
    BMC_CONFIG_DIR,
    SRC_LOGS,
    SRC_ETC_CONF,
    "/etc/nix-upgrade",
    "/etc/nix/nix.conf",
    // additional procfs items
    PROC_MTD,
    PROC_CPUINFO,
];

/// Hosts pinged for the reachability report.
const PING_HOSTS: &[&str] = &[
    "127.0.0.1",
    "8.8.8.8",
    "google.com",
    "downloads.braiins.com",
    "downloads.braiinsforge.com",
    "public-api.braiins.com",
];

/// Credential filters applied to every collected file.
const FILTERS: &[&dyn SupportFilter] = &[&SecretsExclusion, &BmcConfigCensor, &UciWirelessCensor];

/// Device paths plus the shared generic procfs set.
static FS_PATHS: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| [DEVICE_FS_PATHS, bmc_support::PROC_PATHS].concat());

/// Extensions run after the fs walk; `logread` stays last.
const EXTENSIONS: &[&dyn SupportExtension] = &[
    &NixProfileExtension::new(NIX_PROFILE_DIR),
    &LogReadExtension,
];

/// The OpenWrt board's support-archive recipe.
pub static SUPPORT_CONFIG: LazyLock<SupportConfig<'static>> = LazyLock::new(|| {
    SupportConfig::new()
        .commands(COMMANDS)
        .fs_paths(&FS_PATHS)
        .ping_hosts(PING_HOSTS)
        .filters(FILTERS)
        .extensions(EXTENSIONS)
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_config_pins_the_include_set() {
        let commands: Vec<Vec<&str>> = COMMANDS.iter().map(|cmd| cmd.to_vec()).collect();
        assert_eq!(
            commands,
            vec![
                vec!["dmesg"],
                vec!["fw_printenv"],
                vec!["env"],
                vec!["ifconfig", "-a"],
                vec!["ip", "addr"],
                vec!["ip", "route"],
                vec!["ps", "aux"],
                vec!["df"],
                vec!["ls", "-l", "/tmp"],
                vec!["killall", "-SIGUSR1", "dnsmasq"],
            ]
        );

        assert_eq!(
            PING_HOSTS.to_vec(),
            vec![
                "127.0.0.1",
                "8.8.8.8",
                "google.com",
                "downloads.braiins.com",
                "downloads.braiinsforge.com",
                "public-api.braiins.com",
            ]
        );

        assert_eq!(
            FS_PATHS.as_slice(),
            [DEVICE_FS_PATHS, bmc_support::PROC_PATHS]
                .concat()
                .as_slice()
        );

        let extensions: Vec<_> = EXTENSIONS.iter().map(|ext| ext.name()).collect();
        assert_eq!(extensions, ["nix_profile", "logread"]);
    }
}
