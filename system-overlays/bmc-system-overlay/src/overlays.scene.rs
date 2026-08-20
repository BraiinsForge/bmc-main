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

//! Gallery previews for the system overlays.
//!
//! This file is *not* compiled by `bmc-system-overlay`; it is discovered by the
//! gallery build script and compiled into the `bmc-gallery` scenes dylib, whose
//! Cargo manifest provides the `bmc_overlay_*` crates used below (the same
//! arrangement as `bmc-render/src/keyboard.scene.rs`).

use std::cell::RefCell;
use std::net::Ipv4Addr;
use std::thread::LocalKey;
use std::time::Instant;

use bmc_gallery::prelude::*;

use bmc_overlay_alarm::{AlarmRenderState, AlarmView, render_alarm};
use bmc_overlay_device_info::{DeviceInfoRenderState, DeviceInfoView, render_device_info};
use bmc_overlay_offline::{OfflineView, render_offline};
use bmc_overlay_settings_tray::{
    NightModeView, SettingsTrayProduct, SettingsTrayRenderState, SettingsTrayView,
    render_settings_tray,
};
use bmc_overlay_upgrade::{PACKAGE_SURFACE_SIZE, UpgradeRenderState, UpgradeView, render_upgrade};
use bmc_render::colors::Color;
use bmc_render::renderer::Renderer;
use bmc_system_overlay::{DownloadProgress, UpgradeKind, UpgradePhase};

scene_meta! { title: "Overlays" }

/// Deck display dimensions; the fullscreen overlays render at this size.
const DISPLAY_W: u32 = 1_280;
const DISPLAY_H: u32 = 480;

/// What a transparent overlay (the chip, the tray's empty band) is read against:
/// the stage's own backdrop, or a flat fill standing in for a widget beneath.
fn draw_backdrop(r: &mut dyn Renderer, w: f32, h: f32, flat: bool) {
    if flat {
        r.fill_rect(0.0, 0.0, w, h, Color::from_rgba(32, 32, 32, 255));
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
/// night mode, restart, the WiFi hold) on the given product.
fn all_groups_view(base: SettingsTrayView) -> SettingsTrayView {
    let mut view = base;
    view.show_volume = true;
    view.wifi_button = true;
    view.show_restart = true;
    view.night_mode = Some(NightModeView {
        active: true,
        until: Some("06:30".to_owned()),
    });
    view.restart_caption = None;
    view.reconfig_caption = None;
    view
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a stage hands its cell a size in whole positive logical pixels"
)]
fn offline_cell(flat: bool) -> CustomRenderFn {
    Box::new(move |r, _interaction, w, h, _delta| {
        draw_backdrop(r, w, h, flat);
        render_offline(r, (w as u32, h as u32), OfflineView { visible: true });
        // Still: `render_offline` is given no clock to move against.
        false
    })
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a stage hands its cell a size in whole positive logical pixels"
)]
fn device_info_cell(
    view: DeviceInfoView,
    state_key: &'static LocalKey<RefCell<DeviceInfoRenderState>>,
    flat: bool,
) -> CustomRenderFn {
    Box::new(move |r, _interaction, w, h, _delta| {
        draw_backdrop(r, w, h, flat);
        state_key.with_borrow_mut(|state| {
            render_device_info(r, (w as u32, h as u32), state, &view);
        });
        // Still, for the same reason as the offline card: a view and nothing else.
        false
    })
}

/// One retained state per stage, mirroring the upgrade cards:
/// every screen is drawn each frame and holds its own tree/icon caches.
macro_rules! device_info_render_states {
    ($($state:ident),+ $(,)?) => {
        thread_local! {
            $(static $state: RefCell<DeviceInfoRenderState> =
                RefCell::new(DeviceInfoRenderState::new(Instant::now()));)+
        }
    };
}

device_info_render_states!(
    DI_SETUP_START,
    DI_SETUP_START_PENDING,
    DI_SETUP_CONNECTING,
    DI_SETUP_CONNECTED,
    DI_SETUP_CONNECT_INFO,
    DI_SETUP_CONNECT_INFO_PENDING,
    DI_SETUP_COMPLETED,
    DI_SETUP_ERROR,
    DI_SETUP_FATAL_RESTARTING,
    DI_SETUP_FATAL,
    DI_UPGRADE_SUCCESS,
    DI_CONNECTING,
    DI_SUCCESS,
    DI_FAILED,
);

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
    reason = "a stage hands its cell a size in whole positive logical pixels"
)]
fn alarm_cell(
    view: AlarmView,
    state_key: &'static LocalKey<RefCell<AlarmRenderState>>,
    flat: bool,
) -> CustomRenderFn {
    Box::new(move |r, _interaction, w, h, _delta| {
        draw_backdrop(r, w, h, flat);
        state_key.with_borrow_mut(|state| {
            render_alarm(r, (w as u32, h as u32), state, &view);
        });
        // The rings run off a clock of their own, so this never comes to rest.
        true
    })
}

/// One retained state per stage: every phase is drawn each frame,
/// and a shared state would make each draw continue the previous phase's animation.
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
    reason = "a stage hands its cell a size in whole positive logical pixels"
)]
fn upgrade_cell(
    view: UpgradeView,
    state_key: &'static LocalKey<RefCell<UpgradeRenderState>>,
    flat: bool,
) -> CustomRenderFn {
    Box::new(move |r, _interaction, w, h, _delta| {
        draw_backdrop(r, w, h, flat);
        state_key.with_borrow_mut(|state| {
            render_upgrade(r, (w as u32, h as u32), state, &view, Instant::now());
        });
        // Handed a clock of its own, so it can animate — the running phases never settle.
        true
    })
}

/// What a whole section shares: its caption and the surface its cards stage at.
/// The phase is the only thing that varies card to card.
#[derive(Clone, Copy)]
struct Section {
    title: &'static str,
    size: (u32, u32),
}

const FIRMWARE: Section = Section {
    title: "Firmware",
    size: (DISPLAY_W, DISPLAY_H),
};

const PACKAGES: Section = Section {
    title: "Packages",
    size: PACKAGE_SURFACE_SIZE,
};

fn upgrade_stage(
    ctx: &mut SceneCtx,
    ui: &mut Ui,
    section: Section,
    phase: &str,
    view: UpgradeView,
    state: &'static LocalKey<RefCell<UpgradeRenderState>>,
    flat: bool,
) {
    ui.heading(section.title);
    ui.label(phase);
    ctx.custom_stage(ui, section.size, upgrade_cell(view, state, flat));
}

/// What `matrix_with` measures its columns from: one entry per cell,
/// every card in a section being staged at the same size.
#[expect(
    clippy::cast_precision_loss,
    reason = "an overlay surface is a few hundred pixels across"
)]
fn matrix_sizes(size: (u32, u32), count: usize) -> Vec<egui::Vec2> {
    vec![egui::vec2(size.0 as f32, size.1 as f32); count]
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
    static RESTART_HOLDING_ROUND_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
        RefCell::new(SettingsTrayRenderState::new(Instant::now()));
    static RESTART_HOLDING_LARGE_TRAY_RENDER_STATE: RefCell<SettingsTrayRenderState> =
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
    reason = "a stage hands its cell a size in whole positive logical pixels"
)]
fn settings_tray_cell(
    view: SettingsTrayView,
    state_key: &'static LocalKey<RefCell<SettingsTrayRenderState>>,
    flat: bool,
) -> CustomRenderFn {
    Box::new(move |r, _interaction, w, h, _delta| {
        draw_backdrop(r, w, h, flat);
        state_key.with_borrow_mut(|state| {
            let _ = render_settings_tray(r, (w as u32, h as u32), state, &view, Instant::now());
        });
        // Reveal and press feedback are timed off the clock it is handed, and it
        // reports taps rather than whether either is still running.
        true
    })
}

#[scene(default)]
fn offline(ctx: &mut SceneCtx, ui: &mut Ui) {
    let flat = ctx.toggle("Flat backdrop", false);
    ui.heading("Offline");
    ui.label("no routable IPv4");
    ctx.custom_stage(ui, (160_u32, 48_u32), offline_cell(flat));
}

fn device_info_stage(
    ctx: &mut SceneCtx,
    ui: &mut Ui,
    heading: &str,
    caption: &str,
    view: DeviceInfoView,
    state: &'static LocalKey<RefCell<DeviceInfoRenderState>>,
    flat: bool,
) {
    ui.heading(heading);
    ui.label(caption);
    ctx.custom_stage(
        ui,
        (DISPLAY_W, DISPLAY_H),
        device_info_cell(view, state, flat),
    );
}

/// One card per screen of the device-info flows: the first-boot setup sequence,
/// its error states, and the operational connect-info sequence.
#[scene]
#[expect(
    clippy::too_many_lines,
    reason = "a flat catalogue: one stage per screen, which reads worse split \
              across helpers than listed in flow order"
)]
fn device_info(ctx: &mut SceneCtx, ui: &mut Ui) {
    let flat = ctx.toggle("Flat backdrop", false);
    let ap = Some((
        "Braiins Deck AP".to_owned(),
        "http://192.168.8.1/".to_owned(),
    ));

    device_info_stage(
        ctx,
        ui,
        "SetupStart",
        "first boot: AP SSID",
        DeviceInfoView::SetupStart { ap },
        &DI_SETUP_START,
        flat,
    );
    device_info_stage(
        ctx,
        ui,
        "SetupStart (AP pending)",
        "AP still coming up",
        DeviceInfoView::SetupStart { ap: None },
        &DI_SETUP_START_PENDING,
        flat,
    );
    device_info_stage(
        ctx,
        ui,
        "SetupConnecting",
        "joining the chosen network",
        DeviceInfoView::SetupConnecting {
            ssid: Some("Braiins-WiFi".to_owned()),
        },
        &DI_SETUP_CONNECTING,
        flat,
    );
    device_info_stage(
        ctx,
        ui,
        "SetupConnected",
        "network joined",
        DeviceInfoView::SetupConnected {
            ssid: Some("Braiins-WiFi".to_owned()),
        },
        &DI_SETUP_CONNECTED,
        flat,
    );
    device_info_stage(
        ctx,
        ui,
        "SetupConnectInfo",
        "device-setup URL + IP QR",
        DeviceInfoView::SetupConnectInfo {
            ip: Some(Ipv4Addr::new(192, 168, 1, 42)),
            ssid: Some("Braiins-WiFi".to_owned()),
        },
        &DI_SETUP_CONNECT_INFO,
        flat,
    );
    device_info_stage(
        ctx,
        ui,
        "SetupConnectInfo (IP pending)",
        "no station address yet",
        DeviceInfoView::SetupConnectInfo {
            ip: None,
            ssid: Some("Braiins-WiFi".to_owned()),
        },
        &DI_SETUP_CONNECT_INFO_PENDING,
        flat,
    );
    device_info_stage(
        ctx,
        ui,
        "SetupCompleted",
        "wizard finished",
        DeviceInfoView::SetupCompleted,
        &DI_SETUP_COMPLETED,
        flat,
    );
    device_info_stage(
        ctx,
        ui,
        "SetupError",
        "join failed; the AP screen returns",
        DeviceInfoView::SetupError,
        &DI_SETUP_ERROR,
        flat,
    );
    device_info_stage(
        ctx,
        ui,
        "SetupFatal (restarting)",
        "bmc restarts or resets the device",
        DeviceInfoView::SetupFatal { restarting: true },
        &DI_SETUP_FATAL_RESTARTING,
        flat,
    );
    device_info_stage(
        ctx,
        ui,
        "SetupFatal",
        "bmc takes no action; the user has to restart",
        DeviceInfoView::SetupFatal { restarting: false },
        &DI_SETUP_FATAL,
        flat,
    );
    device_info_stage(
        ctx,
        ui,
        "UpgradeSuccess",
        "first boot after a firmware upgrade",
        DeviceInfoView::UpgradeSuccess,
        &DI_UPGRADE_SUCCESS,
        flat,
    );
    device_info_stage(
        ctx,
        ui,
        "Connecting",
        "operational boot, waiting for IP",
        DeviceInfoView::Connecting {
            ssid: Some("Braiins-WiFi".to_owned()),
        },
        &DI_CONNECTING,
        flat,
    );
    device_info_stage(
        ctx,
        ui,
        "Success",
        "IP acquired",
        DeviceInfoView::Success {
            ip: Ipv4Addr::new(192, 168, 1, 42),
        },
        &DI_SUCCESS,
        flat,
    );
    device_info_stage(
        ctx,
        ui,
        "Failed",
        "no IP before timeout",
        DeviceInfoView::Failed {
            ssid: Some("Braiins-WiFi".to_owned()),
        },
        &DI_FAILED,
        flat,
    );
}

/// One tray cell per capability and per new state, in a flat run down the scene.
#[scene]
#[expect(
    clippy::too_many_lines,
    reason = "a flat catalogue: one stage per product and control-group variant, \
              which reads worse split across helpers than listed in order"
)]
fn settings_tray(ctx: &mut SceneCtx, ui: &mut Ui) {
    let flat = ctx.toggle("Flat backdrop", false);

    let bmc100 = bmc100_tray_view();
    ui.heading("Settings tray");
    ui.label("BMC100");
    ctx.custom_stage(
        ui,
        (bmc100.width, bmc100.height),
        settings_tray_cell(bmc100, &BMC100_TRAY_RENDER_STATE, flat),
    );

    let bmm101 = bmm101_tray_view();
    ui.heading("Settings tray");
    ui.label("BMM101");
    ctx.custom_stage(
        ui,
        (bmm101.width, bmm101.height),
        settings_tray_cell(bmm101, &BMM101_TRAY_RENDER_STATE, flat),
    );

    let bmm100 = bmm100_tray_view();
    ui.heading("Settings tray");
    ui.label("BMM100");
    ctx.custom_stage(
        ui,
        (bmm100.width, bmm100.height),
        settings_tray_cell(bmm100, &BMM100_TRAY_RENDER_STATE, flat),
    );

    let bfm100 = bfm100_tray_view();
    ui.heading("Settings tray");
    ui.label("BFM100");
    ctx.custom_stage(
        ui,
        (bfm100.width, bfm100.height),
        settings_tray_cell(bfm100, &BFM100_TRAY_RENDER_STATE, flat),
    );

    let mut night_mode_active = bmc100_tray_view();
    night_mode_active.night_mode = Some(NightModeView {
        active: true,
        until: Some("06:30".to_owned()),
    });
    ui.heading("Settings tray");
    ui.label("Night mode active");
    ctx.custom_stage(
        ui,
        (night_mode_active.width, night_mode_active.height),
        settings_tray_cell(
            night_mode_active,
            &NIGHT_MODE_ACTIVE_TRAY_RENDER_STATE,
            flat,
        ),
    );

    let mut night_mode_inactive = bmc100_tray_view();
    night_mode_inactive.night_mode = Some(NightModeView {
        active: false,
        until: None,
    });
    ui.heading("Settings tray");
    ui.label("Night mode inactive, no schedule");
    ctx.custom_stage(
        ui,
        (night_mode_inactive.width, night_mode_inactive.height),
        settings_tray_cell(
            night_mode_inactive,
            &NIGHT_MODE_INACTIVE_TRAY_RENDER_STATE,
            flat,
        ),
    );

    let mut night_bmm101 = bmm101_tray_view();
    night_bmm101.night_mode = Some(NightModeView {
        active: true,
        until: Some("06:30".to_owned()),
    });
    ui.heading("Settings tray");
    ui.label("Night mode active, BMM101 caption line");
    ctx.custom_stage(
        ui,
        (night_bmm101.width, night_bmm101.height),
        settings_tray_cell(night_bmm101, &NIGHT_MODE_BMM101_TRAY_RENDER_STATE, flat),
    );

    let mut volume_low = bmc100_tray_view();
    volume_low.volume = 0;
    ui.heading("Settings tray");
    ui.label("Volume low");
    ctx.custom_stage(
        ui,
        (volume_low.width, volume_low.height),
        settings_tray_cell(volume_low, &VOLUME_LOW_TRAY_RENDER_STATE, flat),
    );

    let mut volume_high = bmc100_tray_view();
    volume_high.volume = 100;
    ui.heading("Settings tray");
    ui.label("Volume high");
    ctx.custom_stage(
        ui,
        (volume_high.width, volume_high.height),
        settings_tray_cell(volume_high, &VOLUME_HIGH_TRAY_RENDER_STATE, flat),
    );

    let mut pressed = bmc100_tray_view();
    pressed.pressed_key = Some("volume_up".to_owned());
    ui.heading("Settings tray");
    ui.label("Pressed volume-up (inverted disc)");
    ctx.custom_stage(
        ui,
        (pressed.width, pressed.height),
        settings_tray_cell(pressed, &PRESSED_TRAY_RENDER_STATE, flat),
    );

    let mut restart_holding_round = bfm100_tray_view();
    restart_holding_round.restart_progress = 0.15;
    restart_holding_round.restart_caption = Some("Keep holding…".to_owned());
    ui.heading("Settings tray");
    ui.label("Restart holding, round tier at 15%");
    ctx.custom_stage(
        ui,
        (restart_holding_round.width, restart_holding_round.height),
        settings_tray_cell(
            restart_holding_round,
            &RESTART_HOLDING_ROUND_TRAY_RENDER_STATE,
            flat,
        ),
    );

    let mut restart_holding_large = bmc100_tray_view();
    restart_holding_large.restart_progress = 0.15;
    restart_holding_large.restart_caption = Some("Keep holding…".to_owned());
    ui.heading("Settings tray");
    ui.label("Restart holding, Large tier at 15%");
    ctx.custom_stage(
        ui,
        (restart_holding_large.width, restart_holding_large.height),
        settings_tray_cell(
            restart_holding_large,
            &RESTART_HOLDING_LARGE_TRAY_RENDER_STATE,
            flat,
        ),
    );

    let mut restart_declined = bmm100_tray_view();
    restart_declined.restart_caption = Some("upgrade in progress".to_owned());
    ui.heading("Settings tray");
    ui.label("Restart declined, BMM100 caption fit");
    ctx.custom_stage(
        ui,
        (restart_declined.width, restart_declined.height),
        settings_tray_cell(restart_declined, &RESTART_DECLINED_TRAY_RENDER_STATE, flat),
    );

    let all_groups_bfm100 = all_groups_view(bfm100_tray_view());
    ui.heading("Settings tray");
    ui.label("All groups, BFM100 round");
    ctx.custom_stage(
        ui,
        (all_groups_bfm100.width, all_groups_bfm100.height),
        settings_tray_cell(
            all_groups_bfm100,
            &ALL_GROUPS_BFM100_TRAY_RENDER_STATE,
            flat,
        ),
    );

    let all_groups_bmm100 = all_groups_view(bmm100_tray_view());
    ui.heading("Settings tray");
    ui.label("All groups, BMM100 tight budget");
    ctx.custom_stage(
        ui,
        (all_groups_bmm100.width, all_groups_bmm100.height),
        settings_tray_cell(
            all_groups_bmm100,
            &ALL_GROUPS_BMM100_TRAY_RENDER_STATE,
            flat,
        ),
    );

    let mut setup_bmm100 = bmm100_tray_view();
    setup_bmm100.setup_ssid = Some("Braiins-Deck-Setup-A1B2C3".to_owned());
    ui.heading("Settings tray");
    ui.label("Setup mode, BMM100 compact row");
    ctx.custom_stage(
        ui,
        (setup_bmm100.width, setup_bmm100.height),
        settings_tray_cell(setup_bmm100, &SETUP_BMM100_TRAY_RENDER_STATE, flat),
    );
}

#[scene]
fn alarm(ctx: &mut SceneCtx, ui: &mut Ui) {
    let flat = ctx.toggle("Flat backdrop", false);

    ui.heading("Alarm");
    ui.label("with label, snooze allowed");
    ctx.custom_stage(
        ui,
        (DISPLAY_W, DISPLAY_H),
        alarm_cell(
            AlarmView {
                time: "07:30".to_owned(),
                period: String::new(),
                label: "Wake up".to_owned(),
                snooze_allowed: true,
            },
            &ALARM_LABEL_RENDER_STATE,
            flat,
        ),
    );

    ui.heading("Alarm");
    ui.label("no label, snooze disabled");
    ctx.custom_stage(
        ui,
        (DISPLAY_W, DISPLAY_H),
        alarm_cell(
            AlarmView {
                time: "06:00".to_owned(),
                period: String::new(),
                label: String::new(),
                snooze_allowed: false,
            },
            &ALARM_NO_LABEL_RENDER_STATE,
            flat,
        ),
    );

    ui.heading("Alarm");
    ui.label("12-hour time with AM/PM marker");
    ctx.custom_stage(
        ui,
        (DISPLAY_W, DISPLAY_H),
        alarm_cell(
            AlarmView {
                time: "07:30".to_owned(),
                period: "PM".to_owned(),
                label: "Wake up".to_owned(),
                snooze_allowed: true,
            },
            &ALARM_12H_RENDER_STATE,
            flat,
        ),
    );
}

#[scene]
#[expect(
    clippy::too_many_lines,
    reason = "the two phase tables are the scene's content; naming each entry \
              once here beats hiding them behind a builder"
)]
fn upgrade_progress(ctx: &mut SceneCtx, ui: &mut Ui) {
    let flat = ctx.toggle("Flat backdrop", false);

    // A download the server sized, and one it did not:
    // the second drives the indeterminate bar, which has no percentage to show.
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

    let firmware_phases = [
        ("Preparing", firmware(None, None), &FIRMWARE_PREPARING),
        (
            "Known-total download",
            firmware(Some(UpgradePhase::FirmwareDownloading), known),
            &FIRMWARE_KNOWN_DOWNLOAD,
        ),
        (
            "Unknown-total download",
            firmware(Some(UpgradePhase::FirmwareDownloading), unknown),
            &FIRMWARE_UNKNOWN_DOWNLOAD,
        ),
        (
            "Verifying",
            firmware(Some(UpgradePhase::FirmwareVerifying), None),
            &FIRMWARE_VERIFYING,
        ),
        (
            "Package realizing",
            firmware(Some(UpgradePhase::PackageRealizing), None),
            &FIRMWARE_PACKAGES_REALIZING,
        ),
        (
            "Package verifying",
            firmware(Some(UpgradePhase::PackageVerifying), None),
            &FIRMWARE_PACKAGES_VERIFYING,
        ),
        (
            "Package building",
            firmware(Some(UpgradePhase::PackageBuilding), None),
            &FIRMWARE_PACKAGES_BUILDING,
        ),
        (
            "Package activating",
            firmware(Some(UpgradePhase::PackageActivating), None),
            &FIRMWARE_PACKAGES_ACTIVATING,
        ),
        (
            "Applying",
            firmware(Some(UpgradePhase::FirmwareApplying), None),
            &FIRMWARE_APPLYING,
        ),
        (
            "Failure",
            UpgradeView::Failed {
                kind: UpgradeKind::Firmware,
            },
            &FIRMWARE_FAILURE,
        ),
    ];
    let firmware_sizes = matrix_sizes(FIRMWARE.size, firmware_phases.len());
    ctx.matrix_with(ui, &firmware_sizes, |ctx, ui, at| {
        let (phase, view, state) = firmware_phases[at];
        // A grid counts every widget as a column, so the caption rides with its card
        // as one block — the same reason a stage wraps itself.
        ui.vertical(|ui| {
            upgrade_stage(ctx, ui, FIRMWARE, phase, view, state, flat);
        });
    });

    let package_phases = [
        ("Preparing", packages(None, None), &PACKAGE_PREPARING),
        (
            "Known-total download",
            packages(Some(UpgradePhase::PackageRealizing), known),
            &PACKAGE_KNOWN_DOWNLOAD,
        ),
        (
            "Unknown-total download",
            packages(Some(UpgradePhase::PackageRealizing), unknown),
            &PACKAGE_UNKNOWN_DOWNLOAD,
        ),
        (
            "Verifying",
            packages(Some(UpgradePhase::PackageVerifying), None),
            &PACKAGE_VERIFYING,
        ),
        (
            "Building",
            packages(Some(UpgradePhase::PackageBuilding), None),
            &PACKAGE_BUILDING,
        ),
        (
            "Activating",
            packages(Some(UpgradePhase::PackageActivating), None),
            &PACKAGE_ACTIVATING,
        ),
        (
            "Success",
            UpgradeView::Succeeded {
                kind: UpgradeKind::Packages,
            },
            &PACKAGE_SUCCESS,
        ),
        (
            "Failure",
            UpgradeView::Failed {
                kind: UpgradeKind::Packages,
            },
            &PACKAGE_FAILURE,
        ),
    ];
    let package_sizes = matrix_sizes(PACKAGES.size, package_phases.len());
    ctx.matrix_with(ui, &package_sizes, |ctx, ui, at| {
        let (phase, view, state) = package_phases[at];
        ui.vertical(|ui| {
            upgrade_stage(ctx, ui, PACKAGES, phase, view, state, flat);
        });
    });
}
