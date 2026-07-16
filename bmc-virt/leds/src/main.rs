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

// Virtual SPI LED visualization for bmc-virt.

use bmc_virt_leds::run_from_paths;

fn main() {
    let mut args = std::env::args().skip(1);
    let drm_path = args.next().unwrap_or_else(|| "/dev/dri/card1".to_owned());
    let capture_path = args
        .next()
        .unwrap_or_else(|| "/proc/bmc_virt_spi0".to_owned());

    if let Err(e) = run_from_paths(&drm_path, &capture_path) {
        eprintln!("bmc-virt-leds: capture error: {e}");
        std::process::exit(1);
    }
}
