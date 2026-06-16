// Copyright (C) 2026  Braiins Systems s.r.o.

//! Storybook previews for the system overlays.
//!
//! This file is *not* compiled by `bmc-system-overlay`; it is discovered by the
//! storybook build script and compiled into the `bmc-storybook-stories` cdylib,
//! whose Cargo manifest provides the `bmc_overlay_*` crates used below (the same
//! arrangement as `bmc-render/src/keyboard.stories.rs`).

use std::cell::RefCell;
use std::net::Ipv4Addr;

use crate::prelude::*;

use bmc_overlay_device_info::DeviceInfoOverlay;
use bmc_overlay_offline::OfflineOverlay;
use bmc_overlay_settings_tray::SettingsTrayOverlay;
use bmc_render::colors::Color;
use bmc_render::renderer::Renderer;
use bmc_system_overlay::SystemOverlay;

story_meta! { title: "Overlays" }

/// Deck display dimensions; the fullscreen overlays render at this size.
const DISPLAY_W: u32 = 1_280;
const DISPLAY_H: u32 = 480;

/// One persistent instance per scenario, so any per-frame state survives across
/// frames just like on-device.
struct Overlays {
    offline: OfflineOverlay,
    dev_connecting: DeviceInfoOverlay,
    dev_success: DeviceInfoOverlay,
    dev_failed: DeviceInfoOverlay,
    tray: SettingsTrayOverlay,
}

thread_local! {
    static OVERLAYS: RefCell<Overlays> = RefCell::new(Overlays {
        offline: OfflineOverlay::default(),
        dev_connecting: DeviceInfoOverlay::preview_connecting(Some("Braiins-WiFi".to_owned())),
        dev_success: DeviceInfoOverlay::preview_success(Ipv4Addr::new(192, 168, 1, 42)),
        dev_failed: DeviceInfoOverlay::preview_failed(),
        tray: SettingsTrayOverlay::preview_revealed(),
    });
}

#[derive(Clone, Copy)]
enum Which {
    Offline,
    DevConnecting,
    DevSuccess,
    DevFailed,
    Tray,
}

fn pick(o: &mut Overlays, which: Which) -> &mut dyn SystemOverlay {
    match which {
        Which::Offline => &mut o.offline,
        Which::DevConnecting => &mut o.dev_connecting,
        Which::DevSuccess => &mut o.dev_success,
        Which::DevFailed => &mut o.dev_failed,
        Which::Tray => &mut o.tray,
    }
}

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

/// Build a custom-render closure that paints the backdrop then renders the
/// selected overlay at the frame size.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "frame size to logical overlay size"
)]
fn overlay_cell(which: Which, checker: bool) -> CustomRenderFn {
    Box::new(move |r, _interaction, w, h, _delta| {
        draw_backdrop(r, w, h, checker);
        OVERLAYS.with_borrow_mut(|o| pick(o, which).render(r, (w as u32, h as u32)));
    })
}

#[story(default)]
fn offline(ctx: &mut StoryCtx) {
    let checker = ctx.toggle("Backdrop", true).get();
    ctx.ui.grid(1, 16.0, |grid| {
        grid.cell(|ui| {
            ui.header("Offline", "no routable IPv4");
            ui.div_custom((160_u32, 48_u32), overlay_cell(Which::Offline, checker));
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
                overlay_cell(Which::DevConnecting, checker),
            );
        });
        grid.cell(|ui| {
            ui.header("Success", "IP acquired");
            ui.div_custom(
                (DISPLAY_W, DISPLAY_H),
                overlay_cell(Which::DevSuccess, checker),
            );
        });
        grid.cell(|ui| {
            ui.header("Failed", "no IP before timeout");
            ui.div_custom(
                (DISPLAY_W, DISPLAY_H),
                overlay_cell(Which::DevFailed, checker),
            );
        });
    });
}

#[story]
fn settings_tray(ctx: &mut StoryCtx) {
    let checker = ctx.toggle("Backdrop", true).get();
    ctx.ui.grid(1, 16.0, |grid| {
        grid.cell(|ui| {
            ui.header("Settings tray", "revealed");
            ui.div_custom((DISPLAY_W, DISPLAY_H), overlay_cell(Which::Tray, checker));
        });
    });
}
