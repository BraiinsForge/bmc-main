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

//! Build script — links the ARM Linux GNU binaries non-PIE.
//!
//! Under PIE the loader writes ~12k `R_ARM_RELATIVE` relocations into
//! `.data.rel.ro` at startup, COW-ing ~124 kB of RELRO pages into private
//! anonymous RSS per thin instance — pinned RAM on the swapless device.
//! Linking non-PIE bakes absolute addresses at link time, so those pages
//! stay clean file-backed cache shared by all thins.
//! Accepted tradeoff: no ASLR on this binary.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let target_arch =
        std::env::var("CARGO_CFG_TARGET_ARCH").expect("BUG: cargo sets CARGO_CFG_TARGET_ARCH");
    let target_os =
        std::env::var("CARGO_CFG_TARGET_OS").expect("BUG: cargo sets CARGO_CFG_TARGET_OS");
    let target_env =
        std::env::var("CARGO_CFG_TARGET_ENV").expect("BUG: cargo sets CARGO_CFG_TARGET_ENV");
    if target_arch == "arm" && target_os == "linux" && target_env == "gnu" {
        println!("cargo::rustc-link-arg-bins=-no-pie");
    }
}
