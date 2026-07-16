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

pub mod apa102;
pub mod proc_stream;
#[cfg(target_os = "linux")]
pub mod render;

use apa102::Decoder;
use std::io;

pub fn run_from_paths(drm_path: &str, capture_path: &str) -> io::Result<()> {
    eprintln!("bmc-virt-leds: starting (DRM: {drm_path})");

    #[cfg(target_os = "linux")]
    let mut renderer = match render::Renderer::new(drm_path) {
        Ok(r) => {
            eprintln!("bmc-virt-leds: DRM renderer ready");
            Some(r)
        }
        Err(e) => {
            eprintln!(
                "bmc-virt-leds: DRM renderer unavailable ({e}), running without visualization"
            );
            None
        }
    };
    #[cfg(not(target_os = "linux"))]
    let _ = drm_path;

    let mut decoder = Decoder::new();
    let mut on_write = move |data: &[u8]| {
        if let Some(leds) = decoder.feed(data) {
            #[cfg(target_os = "linux")]
            if let Some(ref mut r) = renderer {
                r.render(&leds);
            }
            #[cfg(not(target_os = "linux"))]
            let _ = leds;
        }
    };

    eprintln!("bmc-virt-leds: reading SPI capture from {capture_path}");
    proc_stream::run(capture_path, &mut on_write)
}
