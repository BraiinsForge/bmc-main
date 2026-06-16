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
    SettingsTrayProduct, SettingsTrayRenderState, SettingsTrayView, render_settings_tray,
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
    });
}
