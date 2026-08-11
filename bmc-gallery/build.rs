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

//! Discover the scene files matched by `gallery.toml`'s globs, which it reads itself —
//! so a bare `cargo build` compiles in the scenes a launcher run does.

fn main() {
    gallery_build::discover_from_env();

    // Lets a dev shell point the scenes dylib at its compositor libraries
    // without putting those on `LD_LIBRARY_PATH`; unset elsewhere, so a no-op.
    // The dylib is dlopened, so it carries the rpath rather than the launcher.
    println!("cargo::rerun-if-env-changed=BMC_TEST_RPATH");
    if let Ok(paths) = std::env::var("BMC_TEST_RPATH") {
        for dir in paths.split(':').filter(|dir| !dir.is_empty()) {
            println!("cargo::rustc-link-arg=-Wl,-rpath,{dir}");
        }
    }
}
