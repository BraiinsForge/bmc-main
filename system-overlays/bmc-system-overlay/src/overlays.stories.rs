// Copyright (C) 2026  Braiins Systems s.r.o.

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

use bmc_overlay_device_info::{DeviceInfoView, render_device_info};
use bmc_overlay_offline::{OfflineView, render_offline};
use bmc_overlay_settings_tray::{
    NightModeView, SettingsTrayProduct, SettingsTrayRenderState, SettingsTrayView,
    render_settings_tray,
};
use bmc_render::colors::Color;
use bmc_render::renderer::Renderer;

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
        until: "22:00".to_owned(),
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
        until: "06:30".to_owned(),
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
            until: "06:30".to_owned(),
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
            until: String::new(),
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
            until: "06:30".to_owned(),
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
