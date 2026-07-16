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

use anyhow::Result;
use clap::Parser as _;

use bmc_wasm_thin::args::{Config, RawArgs};

fn main() {
    if let Err(e) = real_main() {
        tracing::error!("{e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    bmc_wasm_thin::logging::init();

    let raw = RawArgs::parse();
    let config = Config::from_raw(raw)?;
    bmc_wasm_thin::run(config)
}
