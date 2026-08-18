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

//! Minimal demonstration of the crate API: assemble a [`SupportConfig`],
//! register an extension, and collect an unencrypted archive. Run with:
//! `cargo run -p bmc-support --example collect_to_zip`.

use anyhow::Result;
use bmc_support::{PlainZip, SupportArchive, SupportConfig, SupportExtension};
use std::fs::File;

/// Example extension: one archive entry with the machine's hostname.
#[derive(Debug)]
struct Hostname;

impl SupportExtension for Hostname {
    fn name(&self) -> &'static str {
        "hostname"
    }

    fn collect(&self, archive: &mut SupportArchive<'_>) -> Result<()> {
        archive.add_cmd_output(&["hostname"])
    }
}

fn main() -> Result<()> {
    let extensions: &[&dyn SupportExtension] = &[&Hostname];
    let config = SupportConfig::new()
        .commands(&[&["uname", "-a"], &["date"]])
        .extensions(extensions);

    let path = std::env::temp_dir().join("support_archive.zip");
    config.collect(&mut File::create(&path)?, &PlainZip, false)?;
    println!("wrote {}", path.display());

    Ok(())
}
