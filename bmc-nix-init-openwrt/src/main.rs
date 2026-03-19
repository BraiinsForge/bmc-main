// Copyright (C) 2026  Braiins Systems s.r.o.

mod linux_drm_platform;

use bmc_nix_init::InitWindow;
use linux_drm_platform::LinuxDrmPlatform;
use slint::platform::software_renderer::RenderingRotation;

fn main() {
    tracing_subscriber::fmt::init();

    let platform = LinuxDrmPlatform::new(480, 272, RenderingRotation::NoRotation);
    slint::platform::set_platform(Box::new(platform))
        .expect("BUG: failed to set Slint platform");

    let window = InitWindow::new().expect("BUG: failed to create init window");
    window.set_status_text("Initializing...".into());

    slint::run_event_loop().expect("BUG: slint event loop failed");
}
