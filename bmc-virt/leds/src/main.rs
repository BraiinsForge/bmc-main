// Copyright (C) 2026  Braiins Systems s.r.o.

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
