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

//! Minimal demonstration of the crate API: assemble a [`SupportConfig`] and
//! collect an unencrypted archive. Run with:
//! `cargo run -p bmc-support --example collect_to_zip`.

use anyhow::Result;
use bmc_support::{PlainZip, SupportConfig};
use std::fs::File;

fn main() -> Result<()> {
    // A minimal, platform-agnostic recipe. `collect` also runs the built-in
    // network probes and a best-effort Nix-profile and log capture.
    let config = SupportConfig::new().commands(&[&["uname", "-a"], &["date"]]);

    let mut file = File::create("support_archive.zip")?;
    config.collect(&mut file, &PlainZip, false)?;

    println!("wrote {file:?}");

    Ok(())
}
