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

//! Storybook previews for the system overlays.
//!
//! This file is *not* compiled by `bmc-system-overlay`; it is discovered by the
//! storybook build script and compiled into the `bmc-storybook-stories` cdylib,
//! whose Cargo manifest provides the `bmc_overlay_*` crates used below (the same
//! arrangement as `bmc-render/src/keyboard.stories.rs`).

use std::cell::RefCell;
use std::net::Ipv4Addr;
use std::thread::LocalKey;
use std::time::Instant;

use crate::prelude::*;

use bmc_overlay_alarm::{AlarmRenderState, AlarmView, render_alarm};
use bmc_overlay_device_info::{DeviceInfoView, render_device_info};
use bmc_overlay_offline::{OfflineView, render_offline};
use bmc_overlay_settings_tray::{
    NightModeView, SettingsTrayProduct, SettingsTrayRenderState, SettingsTrayView,
    render_settings_tray,
};
use bmc_overlay_upgrade::{PACKAGE_SURFACE_SIZE, UpgradeRenderState, UpgradeView, render_upgrade};
use bmc_render::colors::Color;
use bmc_render::renderer::Renderer;
use bmc_system_overlay::{DownloadProgress, UpgradeKind, UpgradePhase};

story_meta! { title: "Overlays" }

/// Deck display dimensions; the fullscreen overlays render at this size.
const DISPLAY_W: u32 = 1_280;
const DISPLAY_H: u32 = 480;

/// A frame backdrop so transparent overlays (the chip, the tray's empty band)
/// are legible: a checkerboard when `checker`, otherwise a flat fill.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "checkerboard tiling over small bounded frame dimensions"
)]
fn draw_backdrop(r: &mut dyn Renderer, w: f32, h: f32, checker: bool) {
    if !checker {
        r.fill_rect(0.0, 0.0, w, h, Color::from_rgba(32, 32, 32, 255));
        return;
    }
    let tile = 24.0;
    let light = Color::from_rgba(60, 60, 60, 255);
    let dark = Color::from_rgba(40, 40, 40, 255);
    let cols = (w / tile).ceil() as usize;
    let rows = (h / tile).ceil() as usize;
    for row in 0..rows {
        for col in 0..cols {
            let color = if (row + col) % 2 == 0 { light } else { dark };
            r.fill_rect(col as f32 * tile, row as f32 * tile, tile, tile, color);
        }
    }
}

fn tray_view(
    product: SettingsTrayProduct,
    hostname: &str,
    ip: &str,
    ssid: &str,
    brightness: u8,
) -> SettingsTrayView {
    let mut view = SettingsTrayView::for_product(product);
    view.brightness = brightness;
    view.hostname = Some(hostname.to_owned());
    view.ip = Some(ip.to_owned());
    view.wifi_signal = Some(-52);
    view.ssid = Some(ssid.to_owned());
    view.volume = 40;
    view.night_mode = Some(NightModeView {
        active: false,
        until: Some("22:00".to_owned()),
    });
    view.show_restart = true;
    view
}

fn bmc100_tray_view() -> SettingsTrayView {
    tray_view(
        SettingsTrayProduct::Bmc100,
        "braiins-deck",
        "192.168.1.42",
        "Braiins-WiFi",
        70,
    )
}

fn bmm101_tray_view() -> SettingsTrayView {
    tray_view(
        SettingsTrayProduct::Bmm101,
        "braiins-mini",
        "10.0.0.42",
        "Workshop-WiFi",
        55,
    )
}

fn bmm100_tray_view() -> SettingsTrayView {
    tray_view(
        SettingsTrayProduct::Bmm100,
        "braiins-micro",
        "10.0.0.99",
        "Garage-WiFi",
        45,
    )
}

fn bfm100_tray_view() -> SettingsTrayView {
    tray_view(
        SettingsTrayProduct::Bfm100,
        "braiins-frame",
        "10.0.0.7",
        "Studio-WiFi",
        60,
    )
}

/// Worst-case view: every control group visible at once (volume, brightness,
/// night mode, restart, both WiFi holds) on the given product.
fn all_groups_view(base: SettingsTrayView) -> SettingsTrayView {
    let mut view = base;
    view.show_volume = true;
    view.wifi_buttons = true;
    view.show_restart = true;
    view.night_mode = Some(NightModeView {
        active: true,
        until: Some("06:30".to_owned()),
    });
    view.restart_caption = None;
    view.reconfig_caption = None;
    view.reconnect_caption = None;
    view
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "storybook frame size to logical overlay size"
)]
fn offline_cell(checker: bool) -> CustomRenderFn {
    Box::new(move |r, _interaction, w, h, _delta| {
        draw_backdrop(r, w, h, checker);
        render_offline(r, (w as u32, h as u32), OfflineView { visible: true });
    })
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "storybook frame size to logical overlay size"
)]
fn device_info_cell(view: DeviceInfoView, checker: bool) -> CustomRenderFn {
    Box::new(move |r, _interaction, w, h, _delta| {
        draw_backdrop(r, w, h, checker);
        render_device_info(r, (w as u32, h as u32), &view);
    })
}

thread_local! {
    static ALARM_LABEL_RENDER_STATE: RefCell<AlarmRenderState> =
        RefCell::new(AlarmRenderState::new(Instant::now()));
    static ALARM_NO_LABEL_RENDER_STATE: RefCell<AlarmRenderState> =
        RefCell::new(AlarmRenderState::new(Instant::now()));
    static ALARM_12H_RENDER_STATE: RefCell<AlarmRenderState> =
        RefCell::new(AlarmRenderState::new(Instant::now()));
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "storybook frame size to logical overlay size"
)]
fn alarm_cell(
    view: AlarmView,
    state_key: &'static LocalKey<RefCell<AlarmRenderState>>,
    checker: bool,
) -> CustomRenderFn {
    Box::new(move |r, _interaction, w, h, _delta| {
        draw_backdrop(r, w, h, checker);
        state_key.with_borrow_mut(|state| {
            render_alarm(r, (w as u32, h as u32), state, &view);
        });
    })
}

macro_rules! upgrade_render_states {
    ($($state:ident),+ $(,)?) => {
        thread_local! {
            $(static $state: RefCell<UpgradeRenderState> =
                RefCell::new(UpgradeRenderState::new(Instant::now()));)+
        }
    };
}

upgrade_render_states!(
    FIRMWARE_PREPARING,
    FIRMWARE_KNOWN_DOWNLOAD,
    FIRMWARE_UNKNOWN_DOWNLOAD,
    FIRMWARE_VERIFYING,
    FIRMWARE_PACKAGES_REALIZING,
    FIRMWARE_PACKAGES_VERIFYING,
    FIRMWARE_PACKAGES_BUILDING,
    FIRMWARE_PACKAGES_ACTIVATING,
    FIRMWARE_APPLYING,
    FIRMWARE_SUCCESS,
    FIRMWARE_FAILURE,
    PACKAGE_PREPARING,
    PACKAGE_KNOWN_DOWNLOAD,
    PACKAGE_UNKNOWN_DOWNLOAD,
    PACKAGE_VERIFYING,
    PACKAGE_BUILDING,
    PACKAGE_ACTIVATING,
    PACKAGE_SUCCESS,
    PACKAGE_FAILURE,
);

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "storybook frame size to logical overlay size"
)]
fn upgrade_cell(
    view: UpgradeView,
    state_key: &'static LocalKey<RefCell<UpgradeRenderState>>,
    checker: bool,
) -> CustomRenderFn {
    Box::new(move |r, _interaction, w, h, _delta| {
        draw_backdrop(r, w, h, checker);
        state_key.with_borrow_mut(|state| {
            render_upgrade(r, (w as u32, h as u32), state, &view, Instant::now());
        });
    })
}

thread_local! {
    static BMC100_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
    static BMM101_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
    static BMM100_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
    static BFM100_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
    static NIGHT_MODE_ACTIVE_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
    static NIGHT_MODE_INACTIVE_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
    static NIGHT_MODE_BMM101_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
    static VOLUME_LOW_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
    static VOLUME_HIGH_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
    static PRESSED_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
    static RESTART_HOLDING_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
    static RESTART_DECLINED_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
    static ALL_GROUPS_BFM100_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
    static ALL_GROUPS_BMM100_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
    static SETUP_BMM100_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "storybook frame size to logical overlay size"
)]
fn settings_tray_cell(
    view: SettingsTrayView,
    state_key: &'static LocalKey<RefCell<SettingsTrayRenderState>>,
    checker: bool,
) -> CustomRenderFn {
    Box::new(move |r, _interaction, w, h, _delta| {
        draw_backdrop(r, w, h, checker);
        state_key.with_borrow_mut(|state| {
            let _ = render_settings_tray(r, (w as u32, h as u32), state, &view, Instant::now());
        });
    })
}

#[story(default)]
fn offline(ctx: &mut StoryCtx) {
    let checker = ctx.toggle("Backdrop", true).get();
    ctx.ui.grid(1, 16.0, |grid| {
        grid.cell(|ui| {
            ui.header("Offline", "no routable IPv4");
            ui.div_custom((160_u32, 48_u32), offline_cell(checker));
        });
    });
}

#[story]
fn device_info(ctx: &mut StoryCtx) {
    let checker = ctx.toggle("Backdrop", true).get();
    ctx.ui.grid(1, 16.0, |grid| {
        grid.cell(|ui| {
            ui.header("Connecting", "waiting for IP");
            ui.div_custom(
                (DISPLAY_W, DISPLAY_H),
                device_info_cell(
                    DeviceInfoView::Connecting {
                        ssid: Some("Braiins-WiFi".to_owned()),
                    },
                    checker,
                ),
            );
        });
        grid.cell(|ui| {
            ui.header("Success", "IP acquired");
            ui.div_custom(
                (DISPLAY_W, DISPLAY_H),
                device_info_cell(
                    DeviceInfoView::Success {
                        ip: Ipv4Addr::new(192, 168, 1, 42),
                    },
                    checker,
                ),
            );
        });
        grid.cell(|ui| {
            ui.header("Failed", "no IP before timeout");
            ui.div_custom(
                (DISPLAY_W, DISPLAY_H),
                device_info_cell(
                    DeviceInfoView::Failed {
                        ssid: Some("Braiins-WiFi".to_owned()),
                    },
                    checker,
                ),
            );
        });
    });
}

#[story]
#[expect(
    clippy::too_many_lines,
    reason = "flat list of one tray cell per capability and per new state"
)]
fn settings_tray(ctx: &mut StoryCtx) {
    let checker = ctx.toggle("Backdrop", true).get();
    ctx.ui.grid(1, 16.0, |grid| {
        let bmc100 = bmc100_tray_view();
        grid.cell(|ui| {
            ui.header("Settings tray", "BMC100");
            ui.div_custom(
                (bmc100.width, bmc100.height),
                settings_tray_cell(bmc100, &BMC100_TRAY_RENDER_STATE, checker),
            );
        });
        let bmm101 = bmm101_tray_view();
        grid.cell(|ui| {
            ui.header("Settings tray", "BMM101");
            ui.div_custom(
                (bmm101.width, bmm101.height),
                settings_tray_cell(bmm101, &BMM101_TRAY_RENDER_STATE, checker),
            );
        });
        let bmm100 = bmm100_tray_view();
        grid.cell(|ui| {
            ui.header("Settings tray", "BMM100");
            ui.div_custom(
                (bmm100.width, bmm100.height),
                settings_tray_cell(bmm100, &BMM100_TRAY_RENDER_STATE, checker),
            );
        });
        let bfm100 = bfm100_tray_view();
        grid.cell(|ui| {
            ui.header("Settings tray", "BFM100");
            ui.div_custom(
                (bfm100.width, bfm100.height),
                settings_tray_cell(bfm100, &BFM100_TRAY_RENDER_STATE, checker),
            );
        });
        let mut night_mode_active = bmc100_tray_view();
        night_mode_active.night_mode = Some(NightModeView {
            active: true,
            until: Some("06:30".to_owned()),
        });
        grid.cell(|ui| {
            ui.header("Settings tray", "Night mode active");
            ui.div_custom(
                (night_mode_active.width, night_mode_active.height),
                settings_tray_cell(
                    night_mode_active,
                    &NIGHT_MODE_ACTIVE_TRAY_RENDER_STATE,
                    checker,
                ),
            );
        });
        let mut night_mode_inactive = bmc100_tray_view();
        night_mode_inactive.night_mode = Some(NightModeView {
            active: false,
            until: None,
        });
        grid.cell(|ui| {
            ui.header("Settings tray", "Night mode inactive, no schedule");
            ui.div_custom(
                (night_mode_inactive.width, night_mode_inactive.height),
                settings_tray_cell(
                    night_mode_inactive,
                    &NIGHT_MODE_INACTIVE_TRAY_RENDER_STATE,
                    checker,
                ),
            );
        });
        let mut night_bmm101 = bmm101_tray_view();
        night_bmm101.night_mode = Some(NightModeView {
            active: true,
            until: Some("06:30".to_owned()),
        });
        grid.cell(|ui| {
            ui.header("Settings tray", "Night mode active, BMM101 caption line");
            ui.div_custom(
                (night_bmm101.width, night_bmm101.height),
                settings_tray_cell(night_bmm101, &NIGHT_MODE_BMM101_TRAY_RENDER_STATE, checker),
            );
        });
        let mut volume_low = bmc100_tray_view();
        volume_low.volume = 0;
        grid.cell(|ui| {
            ui.header("Settings tray", "Volume low");
            ui.div_custom(
                (volume_low.width, volume_low.height),
                settings_tray_cell(volume_low, &VOLUME_LOW_TRAY_RENDER_STATE, checker),
            );
        });
        let mut volume_high = bmc100_tray_view();
        volume_high.volume = 100;
        grid.cell(|ui| {
            ui.header("Settings tray", "Volume high");
            ui.div_custom(
                (volume_high.width, volume_high.height),
                settings_tray_cell(volume_high, &VOLUME_HIGH_TRAY_RENDER_STATE, checker),
            );
        });
        let mut pressed = bmc100_tray_view();
        pressed.pressed_key = Some("volume_up".to_owned());
        grid.cell(|ui| {
            ui.header("Settings tray", "Pressed volume-up (inverted disc)");
            ui.div_custom(
                (pressed.width, pressed.height),
                settings_tray_cell(pressed, &PRESSED_TRAY_RENDER_STATE, checker),
            );
        });
        let mut restart_holding = bmc100_tray_view();
        restart_holding.restart_progress = 0.6;
        restart_holding.restart_caption = Some("Keep holding…".to_owned());
        grid.cell(|ui| {
            ui.header("Settings tray", "Restart holding, 60% ring");
            ui.div_custom(
                (restart_holding.width, restart_holding.height),
                settings_tray_cell(restart_holding, &RESTART_HOLDING_TRAY_RENDER_STATE, checker),
            );
        });
        let mut restart_declined = bmm100_tray_view();
        restart_declined.restart_caption = Some("upgrade in progress".to_owned());
        grid.cell(|ui| {
            ui.header("Settings tray", "Restart declined, BMM100 caption fit");
            ui.div_custom(
                (restart_declined.width, restart_declined.height),
                settings_tray_cell(
                    restart_declined,
                    &RESTART_DECLINED_TRAY_RENDER_STATE,
                    checker,
                ),
            );
        });
        let all_groups_bfm100 = all_groups_view(bfm100_tray_view());
        grid.cell(|ui| {
            ui.header("Settings tray", "All groups, BFM100 round");
            ui.div_custom(
                (all_groups_bfm100.width, all_groups_bfm100.height),
                settings_tray_cell(
                    all_groups_bfm100,
                    &ALL_GROUPS_BFM100_TRAY_RENDER_STATE,
                    checker,
                ),
            );
        });
        let all_groups_bmm100 = all_groups_view(bmm100_tray_view());
        grid.cell(|ui| {
            ui.header("Settings tray", "All groups, BMM100 tight budget");
            ui.div_custom(
                (all_groups_bmm100.width, all_groups_bmm100.height),
                settings_tray_cell(
                    all_groups_bmm100,
                    &ALL_GROUPS_BMM100_TRAY_RENDER_STATE,
                    checker,
                ),
            );
        });
        let mut setup_bmm100 = bmm100_tray_view();
        setup_bmm100.setup_ssid = Some("Braiins-Deck-Setup-A1B2C3".to_owned());
        grid.cell(|ui| {
            ui.header("Settings tray", "Setup mode, BMM100 compact row");
            ui.div_custom(
                (setup_bmm100.width, setup_bmm100.height),
                settings_tray_cell(setup_bmm100, &SETUP_BMM100_TRAY_RENDER_STATE, checker),
            );
        });
    });
}

#[story]
fn alarm(ctx: &mut StoryCtx) {
    let checker = ctx.toggle("Backdrop", true).get();
    ctx.ui.grid(1, 16.0, |grid| {
        grid.cell(|ui| {
            ui.header("Alarm", "with label, snooze allowed");
            ui.div_custom(
                (DISPLAY_W, DISPLAY_H),
                alarm_cell(
                    AlarmView {
                        time: "07:30".to_owned(),
                        period: String::new(),
                        label: "Wake up".to_owned(),
                        snooze_allowed: true,
                    },
                    &ALARM_LABEL_RENDER_STATE,
                    checker,
                ),
            );
        });
        grid.cell(|ui| {
            ui.header("Alarm", "no label, snooze disabled");
            ui.div_custom(
                (DISPLAY_W, DISPLAY_H),
                alarm_cell(
                    AlarmView {
                        time: "06:00".to_owned(),
                        period: String::new(),
                        label: String::new(),
                        snooze_allowed: false,
                    },
                    &ALARM_NO_LABEL_RENDER_STATE,
                    checker,
                ),
            );
        });
        grid.cell(|ui| {
            ui.header("Alarm", "12-hour time with AM/PM marker");
            ui.div_custom(
                (DISPLAY_W, DISPLAY_H),
                alarm_cell(
                    AlarmView {
                        time: "07:30".to_owned(),
                        period: "PM".to_owned(),
                        label: "Wake up".to_owned(),
                        snooze_allowed: true,
                    },
                    &ALARM_12H_RENDER_STATE,
                    checker,
                ),
            );
        });
    });
}

#[story]
#[expect(
    clippy::too_many_lines,
    reason = "one independently retained render state per upgrade design cell"
)]
fn upgrade_progress(ctx: &mut StoryCtx) {
    let checker = ctx.toggle("Backdrop", true).get();
    let known = Some(DownloadProgress {
        downloaded_bytes: 82_000_000,
        total_bytes: Some(151_000_000),
    });
    let unknown = Some(DownloadProgress {
        downloaded_bytes: 82_000_000,
        total_bytes: None,
    });
    let firmware = |phase, progress| UpgradeView::Running {
        kind: UpgradeKind::Firmware,
        phase,
        progress,
    };
    let packages = |phase, progress| UpgradeView::Running {
        kind: UpgradeKind::Packages,
        phase,
        progress,
    };
    ctx.ui.grid(1, 16.0, |grid| {
        let mut firmware_cell = |title, detail, view, state| {
            grid.cell(|ui| {
                ui.header(title, detail);
                ui.div_custom((DISPLAY_W, DISPLAY_H), upgrade_cell(view, state, checker));
            });
        };
        firmware_cell(
            "Firmware",
            "Preparing",
            firmware(None, None),
            &FIRMWARE_PREPARING,
        );
        firmware_cell(
            "Firmware",
            "Known-total download",
            firmware(Some(UpgradePhase::FirmwareDownloading), known),
            &FIRMWARE_KNOWN_DOWNLOAD,
        );
        firmware_cell(
            "Firmware",
            "Unknown-total download",
            firmware(Some(UpgradePhase::FirmwareDownloading), unknown),
            &FIRMWARE_UNKNOWN_DOWNLOAD,
        );
        firmware_cell(
            "Firmware",
            "Verifying",
            firmware(Some(UpgradePhase::FirmwareVerifying), None),
            &FIRMWARE_VERIFYING,
        );
        firmware_cell(
            "Firmware",
            "Package realizing",
            firmware(Some(UpgradePhase::PackageRealizing), None),
            &FIRMWARE_PACKAGES_REALIZING,
        );
        firmware_cell(
            "Firmware",
            "Package verifying",
            firmware(Some(UpgradePhase::PackageVerifying), None),
            &FIRMWARE_PACKAGES_VERIFYING,
        );
        firmware_cell(
            "Firmware",
            "Package building",
            firmware(Some(UpgradePhase::PackageBuilding), None),
            &FIRMWARE_PACKAGES_BUILDING,
        );
        firmware_cell(
            "Firmware",
            "Package activating",
            firmware(Some(UpgradePhase::PackageActivating), None),
            &FIRMWARE_PACKAGES_ACTIVATING,
        );
        firmware_cell(
            "Firmware",
            "Applying",
            firmware(Some(UpgradePhase::FirmwareApplying), None),
            &FIRMWARE_APPLYING,
        );
        firmware_cell(
            "Firmware",
            "Success",
            UpgradeView::Succeeded {
                kind: UpgradeKind::Firmware,
            },
            &FIRMWARE_SUCCESS,
        );
        firmware_cell(
            "Firmware",
            "Failure",
            UpgradeView::Failed {
                kind: UpgradeKind::Firmware,
            },
            &FIRMWARE_FAILURE,
        );

        let mut package_cell = |title, detail, view, state| {
            grid.cell(|ui| {
                ui.header(title, detail);
                ui.div_custom(PACKAGE_SURFACE_SIZE, upgrade_cell(view, state, checker));
            });
        };
        package_cell(
            "Packages",
            "Preparing",
            packages(None, None),
            &PACKAGE_PREPARING,
        );
        package_cell(
            "Packages",
            "Known-total download",
            packages(Some(UpgradePhase::PackageRealizing), known),
            &PACKAGE_KNOWN_DOWNLOAD,
        );
        package_cell(
            "Packages",
            "Unknown-total download",
            packages(Some(UpgradePhase::PackageRealizing), unknown),
            &PACKAGE_UNKNOWN_DOWNLOAD,
        );
        package_cell(
            "Packages",
            "Verifying",
            packages(Some(UpgradePhase::PackageVerifying), None),
            &PACKAGE_VERIFYING,
        );
        package_cell(
            "Packages",
            "Building",
            packages(Some(UpgradePhase::PackageBuilding), None),
            &PACKAGE_BUILDING,
        );
        package_cell(
            "Packages",
            "Activating",
            packages(Some(UpgradePhase::PackageActivating), None),
            &PACKAGE_ACTIVATING,
        );
        package_cell(
            "Packages",
            "Success",
            UpgradeView::Succeeded {
                kind: UpgradeKind::Packages,
            },
            &PACKAGE_SUCCESS,
        );
        package_cell(
            "Packages",
            "Failure",
            UpgradeView::Failed {
                kind: UpgradeKind::Packages,
            },
            &PACKAGE_FAILURE,
        );
    });
}
