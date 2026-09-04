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

//! The OpenWrt board's support-archive recipe: the shared Braiins OS,
//! OpenWrt and bmc include sets, ping hosts, credential filters and
//! extensions, assembled into the [`SupportConfig`] the manager collects
//! through.

use bmc_support::{
    BOS_COMMANDS, BOS_FS_PATHS, PROC_PATHS, SupportConfig, SupportExtension, SupportFilter,
};
use bmc_support_openwrt::{
    BMC_FS_PATHS, BmcConfigCensor, LogReadExtension, NixProfileExtension, OPENWRT_FS_PATHS,
    SecretsExclusion, UciWirelessCensor,
};
use std::sync::LazyLock;

const NIX_PROFILE_DIR: &str = "/nix/var/nix/gcroots/profiles/bmc";

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

/// The Braiins OS, OpenWrt and bmc include sets plus the generic procfs set.
static FS_PATHS: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| [BOS_FS_PATHS, OPENWRT_FS_PATHS, BMC_FS_PATHS, PROC_PATHS].concat());

/// Extensions run after the fs walk; `logread` stays last.
const EXTENSIONS: &[&dyn SupportExtension] = &[
    &NixProfileExtension::new(NIX_PROFILE_DIR),
    &LogReadExtension,
];

/// The OpenWrt board's support-archive recipe.
pub static SUPPORT_CONFIG: LazyLock<SupportConfig<'static>> = LazyLock::new(|| {
    SupportConfig::new()
        .commands(BOS_COMMANDS)
        .fs_paths(&FS_PATHS)
        .ping_hosts(PING_HOSTS)
        .filters(FILTERS)
        .extensions(EXTENSIONS)
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logread_runs_last() {
        let extensions: Vec<_> = EXTENSIONS.iter().map(|ext| ext.name()).collect();
        assert_eq!(extensions, ["nix_profile", "logread"]);
    }
}
