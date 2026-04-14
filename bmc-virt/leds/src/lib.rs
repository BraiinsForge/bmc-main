// Copyright (C) 2026  Braiins Systems s.r.o.

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
