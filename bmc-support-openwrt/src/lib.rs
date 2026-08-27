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

//! Support-archive pieces shared by every binary running on the OpenWrt board:
//! credential filters for its config layout, plus the Nix profile and system log collectors.
//! Each binary assembles them into its own [`bmc_support::SupportConfig`].

mod filters;
mod nix_profile;

pub use filters::{
    BMC_CONFIG_DIR, BMC_CONFIG_LEGACY, BmcConfigCensor, SecretsExclusion, UciWirelessCensor,
};
pub use nix_profile::NixProfileExtension;

use anyhow::Result;
use bmc_support::{SupportArchive, SupportExtension};

/// Captures the in-memory system log. Register it last so it sees syslog
/// every other collector emits during collection.
#[derive(Debug)]
pub struct LogReadExtension;

impl SupportExtension for LogReadExtension {
    fn name(&self) -> &'static str {
        "logread"
    }

    fn collect(&self, archive: &mut SupportArchive<'_>) -> Result<()> {
        archive.add_cmd_output(&["logread"])
    }
}
