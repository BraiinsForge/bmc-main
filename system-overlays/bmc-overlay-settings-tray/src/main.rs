// Copyright (C) 2026  Braiins Systems s.r.o.

fn main() -> anyhow::Result<()> {
    bmc_system_overlay::run_standalone(Box::new(
        bmc_overlay_settings_tray::SettingsTrayOverlay::default(),
    ))
}
