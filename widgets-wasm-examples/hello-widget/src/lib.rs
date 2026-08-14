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

#![allow(clippy::cast_precision_loss)]
#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        dead_code,
        reason = "native builds exercise pure layout selection; the render tree is used by the wasm entrypoint"
    )
)]

#[cfg_attr(not(test), expect(clippy::wildcard_imports))]
use bmc_wasm_sdk::*;

use std::cell::{Cell, RefCell};
use std::f32::consts::{FRAC_PI_2, TAU};

use AnimProperty::{Rotate, Scale};
use Easing::{EaseInOut, EaseOut, Linear};
use LoopMode::{Forever, PingPong};

const STAR: Svg = include_svg!("assets/star.svg");
const SETTINGS: Svg = include_svg!("assets/settings.svg");
const CHECKMARK: Svg = include_svg!("assets/checkmark.svg");
const WARNING: Svg = include_svg!("assets/warning.svg");
const SEARCH: Svg = include_svg!("assets/search.svg");

thread_local! {
    static COUNTS: RefCell<[u32; 4]> = const { RefCell::new([0; 4]) };
    static MODAL_OPEN: Cell<bool> = const { Cell::new(false) };
}

const BG_COLOR: Color = Color::from_hex(0x66_23_47);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelloLayout {
    SmallCompact,
    MediumCompact,
    LargeCompact,
    FullShowcase,
    RoundShowcase,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompactControls {
    TwoCounterButtons,
    SmallAnimationClockModalAndSubsetLedEffects,
    MediumButtonsModalLedEffectsIconShowcaseAndAnimatedText,
    LargeButtonsModalColorsIconsLedEffectsAndRichText,
    FullShowcase,
    RoundClockControlsAndLedEffects,
}

/// Shape decides before size does: `SizeVariant` names the deck's rectangular
/// slots, and a round display is none of them however its pixels measure.
fn hello_layout(variant: SizeVariant, shape: ViewportShape) -> HelloLayout {
    match shape {
        ViewportShape::Round => HelloLayout::RoundShowcase,
        ViewportShape::Rectangular => match variant {
            SizeVariant::Small => HelloLayout::SmallCompact,
            SizeVariant::Medium => HelloLayout::MediumCompact,
            SizeVariant::Large => HelloLayout::LargeCompact,
            SizeVariant::Full => HelloLayout::FullShowcase,
        },
    }
}

#[cfg(test)]
fn compact_controls(layout: HelloLayout) -> CompactControls {
    match layout {
        HelloLayout::SmallCompact => CompactControls::SmallAnimationClockModalAndSubsetLedEffects,
        HelloLayout::MediumCompact => {
            CompactControls::MediumButtonsModalLedEffectsIconShowcaseAndAnimatedText
        }
        HelloLayout::LargeCompact => {
            CompactControls::LargeButtonsModalColorsIconsLedEffectsAndRichText
        }
        HelloLayout::FullShowcase => CompactControls::FullShowcase,
        HelloLayout::RoundShowcase => CompactControls::RoundClockControlsAndLedEffects,
    }
}

// ---------------------------------------------------------------------------
// Sections — each returns a Node, composed in render() like React components
// ---------------------------------------------------------------------------

fn buttons_section(counts: [u32; 4]) -> Node {
    col(
        props!(gap: 8.0, flex: 1.0),
        [
            text("Buttons", style!(size: 20, color: GRAY_10)),
            row(
                props!(gap: 8.0),
                [
                    button!("primary", fmt!("Primary {}", counts[0]), style: Primary),
                    button!("secondary", fmt!("Secondary {}", counts[1]), style: Secondary),
                ],
            ),
            row(
                props!(gap: 8.0),
                [
                    button!("tertiary", fmt!("Tertiary {}", counts[2]), style: Tertiary),
                    button!("danger", fmt!("Danger {}", counts[3]), style: Danger),
                ],
            ),
            spacer(1.0),
            button!("open_modal", "Open Modal", style: Primary),
        ],
    )
}

fn icon_buttons_section() -> Node {
    col(
        props!(gap: 8.0),
        [
            text("Icon Buttons", style!(size: 16, color: GRAY_10)),
            row(
                props!(gap: 8.0),
                [
                    button!("settings", "Settings", style: Primary, icon: tree::ensure_registered(&SETTINGS)),
                    button!("apply", "Apply", style: Secondary, icon: tree::ensure_registered(&CHECKMARK)),
                ],
            ),
            row(
                props!(gap: 8.0),
                [
                    button!("search", "", style: Secondary, icon: tree::ensure_registered(&SEARCH)),
                    button!("warning", "", style: Danger, icon: tree::ensure_registered(&WARNING)),
                    button!("close", "", style: Primary, icon: ICON_CLOSE),
                ],
            ),
        ],
    )
}

fn animations_section(time: &SystemTime) -> Node {
    let secs = system::current()
        .timezone()
        .and_then(|name| time.local(&Tz::from_runtime(name)))
        .unwrap_or_else(|| time.utc())
        .seconds_since_midnight() as f32;
    let second_angle = secs / 60.0 * TAU;
    let minute_angle = secs / 3_600.0 * TAU;
    let hour_angle = secs / 43_200.0 * TAU;

    col(
        props!(gap: 6.0, flex: 1.0),
        [
            text("Animations", style!(size: 20, color: GRAY_10)),
            text("Pulse + Spin + Color", style!(size: 14, color: GRAY_40)),
            pulse_canvas(64.0),
            text("Clock", style!(size: 14, color: GRAY_40)),
            clock_canvas(128.0, hour_angle, minute_angle, second_angle),
        ],
    )
}

fn pulse_canvas(size: f32) -> Node {
    canvas(
        props!(width: size, height: size),
        [Draw::centered(
            Draw::rect(0.0, 0.0, size * 0.5, size * 0.5, RED_50)
                .animate(Rotate, 0.0, TAU, 4_000, Linear, Forever)
                .animate(Scale, 0.5, 1.0, 1_000, EaseInOut, PingPong)
                .animate_color(RED_50, GREEN_50, 3_000, EaseInOut, PingPong),
        )],
    )
}

fn clock_canvas(size: f32, hour_angle: f32, minute_angle: f32, second_angle: f32) -> Node {
    let c = size / 2.0;
    let mark_radius = size * 0.421_875;
    let hour_top = size * 0.25;
    let hour_height = size * 0.281_25;
    let minute_top = size * 0.156_25;
    let minute_height = size * 0.375;
    let second_top = size * 0.093_75;
    let second_height = size * 0.437_5;

    let mut draws: Vec<Draw> = Vec::with_capacity(18);

    // Circle background
    draws.push(Draw::circle(c, c, c, GRAY_80));

    // 12 hour marks
    for i in 0..12 {
        let angle = i as f32 / 12.0 * TAU - FRAC_PI_2;
        draws.push(Draw::orbit(
            mark_radius,
            angle,
            Draw::rect(0.0, 0.0, 3.0, 3.0, GRAY_50),
        ));
    }

    // Hands: rotated() pivots around canvas center
    // Hour hand
    draws.push(
        Draw::rotated(
            hour_angle,
            Draw::rect(c - 3.0, c - hour_top, 6.0, hour_height, GRAY_10),
        )
        .transition("hour-hand", 500, EaseOut),
    );
    // Minute hand
    draws.push(
        Draw::rotated(
            minute_angle,
            Draw::rect(c - 2.0, c - minute_top, 4.0, minute_height, GRAY_30),
        )
        .transition("minute-hand", 500, EaseOut),
    );
    // Second hand
    draws.push(
        Draw::rotated(
            second_angle,
            Draw::rect(c - 1.0, c - second_top, 1.0, second_height, RED_50),
        )
        .transition("second-hand", 200, EaseOut),
    );
    // Center dot
    draws.push(Draw::centered(Draw::rect(0.0, 0.0, 8.0, 8.0, GRAY_10)));

    canvas(props!(width: size, height: size), draws)
}

fn icons_section() -> Node {
    col(
        props!(gap: 12.0, flex: 1.0),
        [
            text("Icons", style!(size: 20, color: GRAY_10)),
            text("Custom (include_svg!)", style!(size: 12, color: GRAY_50)),
            canvas(
                props!(width: 180.0, height: 40.0),
                [
                    Draw::svg(0.0, 4.0, 32.0, 32.0, &STAR, VIOLET_50),
                    Draw::svg(40.0, 4.0, 32.0, 32.0, &SETTINGS, GREEN_50),
                    Draw::svg(80.0, 4.0, 32.0, 32.0, &CHECKMARK, RED_50),
                    Draw::svg(120.0, 4.0, 32.0, 32.0, &WARNING, ORANGE_50),
                ],
            ),
            text("Built-in (svg_builtin)", style!(size: 12, color: GRAY_50)),
            canvas(
                props!(width: 100.0, height: 40.0),
                [
                    Draw::svg_builtin(0.0, 4.0, 32.0, 32.0, ICON_CLOSE, GRAY_10),
                    Draw::svg_builtin(40.0, 4.0, 32.0, 32.0, ICON_CLOSE, RED_50),
                ],
            ),
            text("Animated", style!(size: 12, color: GRAY_50)),
            canvas(
                props!(width: 64.0, height: 64.0),
                [Draw::centered(
                    Draw::svg(0.0, 0.0, 32.0, 32.0, &STAR, ORANGE_50)
                        .animate(Rotate, 0.0, TAU, 3_000, Linear, Forever)
                        .animate(Scale, 0.6, 1.0, 1_500, EaseInOut, PingPong),
                )],
            ),
        ],
    )
}

fn colors_section() -> Node {
    col(
        props!(gap: 12.0, flex: 1.0),
        [
            text("Colors", style!(size: 20, color: GRAY_10)),
            text("Brand", style!(size: 12, color: GRAY_50)),
            canvas(
                props!(width: 180.0, height: 40.0),
                [
                    Draw::rect(0.0, 0.0, 36.0, 36.0, VIOLET_50),
                    Draw::rect(44.0, 0.0, 36.0, 36.0, GREEN_50),
                    Draw::rect(88.0, 0.0, 36.0, 36.0, RED_50),
                    Draw::rect(132.0, 0.0, 36.0, 36.0, ORANGE_50),
                ],
            ),
            text("Grays", style!(size: 12, color: GRAY_50)),
            canvas(
                props!(width: 180.0, height: 28.0),
                [
                    Draw::rect(0.0, 0.0, 28.0, 24.0, GRAY_10),
                    Draw::rect(36.0, 0.0, 28.0, 24.0, GRAY_30),
                    Draw::rect(72.0, 0.0, 28.0, 24.0, GRAY_50),
                    Draw::rect(108.0, 0.0, 28.0, 24.0, GRAY_70),
                    Draw::rect(144.0, 0.0, 28.0, 24.0, GRAY_90),
                ],
            ),
        ],
    )
}

fn rich_text_section() -> Node {
    col(
        props!(flex: 1.0, gap: 8.0, background: GRAY_90.with_alpha(0.9), padding: 12.0),
        [
            text("Rich Text", style!(size: 20, color: GRAY_10)),
            row(
                props!(gap: 24.0),
                [text_wrapping_demo(), text_styles_demo()],
            ),
        ],
    )
}

fn text_wrapping_demo() -> Node {
    col(
        props!(flex: 1.0, gap: 8.0),
        [
            text("Text Wrapping", style!(size: 12, color: GRAY_50)),
            paragraph(
                style!(size: 14, line_height: 1.4),
                [
                    span("Lorem ipsum dolor sit amet, ", ()),
                    span(
                        "consectetur adipiscing elit",
                        style!(weight: FontWeight::BOLD),
                    ),
                    span(
                        ". Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea ",
                        (),
                    ),
                    span("commodo consequat", style!(color: VIOLET_50)),
                    span(". Duis aute irure dolor in reprehenderit.", ()),
                ],
            ),
        ],
    )
}

fn text_styles_demo() -> Node {
    col(
        props!(flex: 1.0, gap: 8.0),
        [
            text("Text Styles", style!(size: 12, color: GRAY_50)),
            paragraph(
                style!(size: 14, line_height: 1.5),
                [
                    span("Normal, ", ()),
                    span("bold", style!(weight: FontWeight::BOLD)),
                    span(", ", ()),
                    span("italic", style!(italic: true)),
                    span(", ", ()),
                    span("underline", style!(underline: true)),
                    span(", ", ()),
                    span("strikethrough", style!(strikethrough: true)),
                    span(", and ", ()),
                    span("colored", style!(color: GREEN_50, weight: FontWeight::BOLD)),
                    span(" text.", ()),
                ],
            ),
        ],
    )
}

fn led_section() -> Node {
    col(
        props!(flex: 1.0, gap: 4.0, background: GRAY_90, padding: 8.0,
            cross_align: CrossAlign::Center),
        [
            text("LED Effects", style!(size: 14, weight: FontWeight::BOLD)),
            row(
                props!(gap: 4.0, background: GRAY_90, padding: 8.0, wrap: true, cross_align: CrossAlign::Center),
                [
                    button!("led_solid", "Solid Red", size: Small),
                    button!("led_breathe", "Breathe Green", size: Small),
                    button!("led_chase", "Chase Blue", size: Small),
                    button!("led_knight", "Knight Rider", size: Small),
                    button!("led_snake", "Snake Cyan", size: Small),
                    button!("led_off", "LEDs Off", style: Secondary, size: Small),
                ],
            ),
        ],
    )
}

fn medium_controls_section(counts: [u32; 4]) -> Node {
    row(
        props!(gap: 8.0, flex: 1.0, cross_align: CrossAlign::Center),
        [
            col(
                props!(gap: 5.0),
                [
                    row(
                        props!(gap: 5.0),
                        [
                            button!("primary", fmt!("Primary {}", counts[0]), style: Primary, size: Small),
                            button!("secondary", fmt!("Secondary {}", counts[1]), style: Secondary, size: Small),
                        ],
                    ),
                    row(
                        props!(gap: 5.0),
                        [
                            button!("tertiary", fmt!("Tertiary {}", counts[2]), style: Tertiary, size: Small),
                            button!("danger", fmt!("Danger {}", counts[3]), style: Danger, size: Small),
                        ],
                    ),
                ],
            ),
            col(
                props!(gap: 5.0),
                [
                    row(
                        props!(gap: 5.0),
                        [
                            button!("open_modal", "Open Modal", style: Primary, size: Small),
                            button!("settings", "", style: Primary, size: Small, icon: tree::ensure_registered(&SETTINGS)),
                        ],
                    ),
                    row(
                        props!(gap: 5.0),
                        [
                            button!("apply", "", style: Secondary, size: Small, icon: tree::ensure_registered(&CHECKMARK)),
                            button!("search", "", style: Secondary, size: Small, icon: tree::ensure_registered(&SEARCH)),
                        ],
                    ),
                ],
            ),
        ],
    )
}

fn small_controls_section(counts: [u32; 4]) -> Node {
    row(
        props!(gap: 5.0, wrap: true, cross_align: CrossAlign::Center),
        [
            button!("primary", fmt!("Primary {}", counts[0]), style: Primary, size: Small),
            button!("secondary", fmt!("Secondary {}", counts[1]), style: Secondary, size: Small),
            button!("open_modal", "Open Modal", style: Primary, size: Small),
        ],
    )
}

fn small_led_effects_section() -> Node {
    row(
        props!(gap: 5.0, background: GRAY_90.with_alpha(0.45), padding: 5.0, wrap: true, cross_align: CrossAlign::Center),
        [
            text(
                "LED",
                style!(size: 11, weight: FontWeight::BOLD, color: GRAY_40),
            ),
            button!("led_solid", "Solid", size: Small),
            button!("led_breathe", "Breathe", size: Small),
            button!("led_knight", "Knight", size: Small),
            button!("led_off", "Off", style: Secondary, size: Small),
        ],
    )
}

fn small_clock(time: &SystemTime, size: f32) -> Node {
    let secs = system::current()
        .timezone()
        .and_then(|name| time.local(&Tz::from_runtime(name)))
        .unwrap_or_else(|| time.utc())
        .seconds_since_midnight() as f32;
    let second_angle = secs / 60.0 * TAU;
    let minute_angle = secs / 3_600.0 * TAU;
    let hour_angle = secs / 43_200.0 * TAU;

    clock_canvas(size, hour_angle, minute_angle, second_angle)
}

fn small_panel(child: Node) -> Node {
    center(
        props!(flex: 1.0, background: GRAY_90.with_alpha(0.45), padding: 4.0),
        [child],
    )
}

fn medium_led_effects_section() -> Node {
    row(
        props!(gap: 5.0, background: GRAY_90.with_alpha(0.45), padding: 6.0, wrap: true, cross_align: CrossAlign::Center),
        [
            text(
                "LED",
                style!(size: 12, weight: FontWeight::BOLD, color: GRAY_40),
            ),
            button!("led_solid", "Solid", size: Small),
            button!("led_breathe", "Breathe", size: Small),
            button!("led_chase", "Chase", size: Small),
            button!("led_knight", "Knight", size: Small),
            button!("led_snake", "Snake", size: Small),
            button!("led_off", "Off", style: Secondary, size: Small),
        ],
    )
}

fn medium_icon_showcase_section() -> Node {
    row(
        props!(gap: 12.0, background: GRAY_90.with_alpha(0.45), padding: 6.0, cross_align: CrossAlign::Center),
        [
            text(
                "Icons",
                style!(size: 12, weight: FontWeight::BOLD, color: GRAY_40),
            ),
            canvas(
                props!(width: 164.0, height: 28.0),
                [
                    Draw::svg(0.0, 0.0, 28.0, 28.0, &STAR, ORANGE_50),
                    Draw::svg(34.0, 0.0, 28.0, 28.0, &SETTINGS, GREEN_50),
                    Draw::svg(68.0, 0.0, 28.0, 28.0, &CHECKMARK, RED_50),
                    Draw::svg(102.0, 0.0, 28.0, 28.0, &WARNING, VIOLET_50),
                    Draw::svg_builtin(136.0, 0.0, 28.0, 28.0, ICON_CLOSE, GRAY_10),
                ],
            ),
            text("Animated", style!(size: 12, color: GRAY_40)),
            canvas(
                props!(width: 34.0, height: 28.0),
                [Draw::centered(
                    Draw::svg(0.0, 0.0, 24.0, 24.0, &STAR, ORANGE_50)
                        .animate(Rotate, 0.0, TAU, 3_000, Linear, Forever)
                        .animate(Scale, 0.6, 1.0, 1_500, EaseInOut, PingPong),
                )],
            ),
        ],
    )
}

fn large_controls_section(counts: [u32; 4]) -> Node {
    col(
        props!(gap: 6.0, background: GRAY_90.with_alpha(0.45), padding: 8.0),
        [row(
            props!(gap: 6.0, wrap: true, cross_align: CrossAlign::Center),
            [
                button!("primary", fmt!("Primary {}", counts[0]), style: Primary, size: Small),
                button!("secondary", fmt!("Secondary {}", counts[1]), style: Secondary, size: Small),
                button!("tertiary", fmt!("Tertiary {}", counts[2]), style: Tertiary, size: Small),
                button!("danger", fmt!("Danger {}", counts[3]), style: Danger, size: Small),
                button!("open_modal", "Open Modal", style: Primary, size: Small),
            ],
        )],
    )
}

fn large_colors_and_icons_section() -> Node {
    row(
        props!(gap: 10.0, background: GRAY_90.with_alpha(0.45), padding: 8.0, cross_align: CrossAlign::Center),
        [
            col(
                props!(gap: 4.0, flex: 1.0),
                [
                    text("Colors", style!(size: 12, color: GRAY_40)),
                    canvas(
                        props!(width: 180.0, height: 28.0),
                        [
                            Draw::rect(0.0, 0.0, 28.0, 24.0, VIOLET_50),
                            Draw::rect(36.0, 0.0, 28.0, 24.0, GREEN_50),
                            Draw::rect(72.0, 0.0, 28.0, 24.0, RED_50),
                            Draw::rect(108.0, 0.0, 28.0, 24.0, ORANGE_50),
                            Draw::rect(144.0, 0.0, 28.0, 24.0, GRAY_50),
                        ],
                    ),
                ],
            ),
            row(
                props!(gap: 6.0, wrap: true, cross_align: CrossAlign::Center),
                [
                    button!("settings", "Settings", style: Primary, size: Small, icon: tree::ensure_registered(&SETTINGS)),
                    button!("apply", "Apply", style: Secondary, size: Small, icon: tree::ensure_registered(&CHECKMARK)),
                    button!("search", "", style: Secondary, size: Small, icon: tree::ensure_registered(&SEARCH)),
                    button!("warning", "", style: Danger, size: Small, icon: tree::ensure_registered(&WARNING)),
                    button!("close", "", style: Primary, size: Small, icon: ICON_CLOSE),
                ],
            ),
        ],
    )
}

fn large_icon_showcase_section() -> Node {
    row(
        props!(gap: 12.0, background: GRAY_90.with_alpha(0.45), padding: 8.0, cross_align: CrossAlign::Center),
        [
            col(
                props!(gap: 4.0, flex: 1.0),
                [
                    text("Custom Icons", style!(size: 12, color: GRAY_40)),
                    canvas(
                        props!(width: 180.0, height: 34.0),
                        [
                            Draw::svg(0.0, 1.0, 32.0, 32.0, &STAR, ORANGE_50),
                            Draw::svg(38.0, 1.0, 32.0, 32.0, &SETTINGS, GREEN_50),
                            Draw::svg(76.0, 1.0, 32.0, 32.0, &CHECKMARK, RED_50),
                            Draw::svg(114.0, 1.0, 32.0, 32.0, &WARNING, VIOLET_50),
                        ],
                    ),
                ],
            ),
            col(
                props!(gap: 4.0, cross_align: CrossAlign::Center),
                [
                    text("Built-in", style!(size: 12, color: GRAY_40)),
                    canvas(
                        props!(width: 72.0, height: 34.0),
                        [
                            Draw::svg_builtin(0.0, 1.0, 32.0, 32.0, ICON_CLOSE, GRAY_10),
                            Draw::svg_builtin(38.0, 1.0, 32.0, 32.0, ICON_CLOSE, RED_50),
                        ],
                    ),
                ],
            ),
            col(
                props!(gap: 4.0, cross_align: CrossAlign::Center),
                [
                    text("Animated Icon", style!(size: 12, color: GRAY_40)),
                    canvas(
                        props!(width: 48.0, height: 34.0),
                        [Draw::centered(
                            Draw::svg(0.0, 0.0, 28.0, 28.0, &STAR, ORANGE_50)
                                .animate(Rotate, 0.0, TAU, 3_000, Linear, Forever)
                                .animate(Scale, 0.6, 1.0, 1_500, EaseInOut, PingPong),
                        )],
                    ),
                ],
            ),
        ],
    )
}

fn large_led_effects_section() -> Node {
    row(
        props!(gap: 6.0, background: GRAY_90.with_alpha(0.45), padding: 8.0, wrap: true, cross_align: CrossAlign::Center),
        [
            text(
                "LED",
                style!(size: 12, weight: FontWeight::BOLD, color: GRAY_40),
            ),
            button!("led_solid", "Solid Red", size: Small),
            button!("led_breathe", "Breathe Green", size: Small),
            button!("led_chase", "Chase Blue", size: Small),
            button!("led_knight", "Knight Rider", size: Small),
            button!("led_snake", "Snake Cyan", size: Small),
            button!("led_off", "LEDs Off", style: Secondary, size: Small),
        ],
    )
}

fn large_rich_text_section() -> Node {
    col(
        props!(gap: 4.0, background: GRAY_90.with_alpha(0.45), padding: 8.0),
        [
            text("Rich Text", style!(size: 12, color: GRAY_40)),
            text_styles_demo(),
        ],
    )
}

fn compact_animation(size: f32, label: &str, label_size: u32) -> Node {
    col(
        props!(gap: 8.0, cross_align: CrossAlign::Center),
        [
            pulse_canvas(size),
            text(label, style!(size: label_size, color: GRAY_40)),
        ],
    )
}

fn compact_clock(time: &SystemTime, size: f32, label_size: u32) -> Node {
    let secs = system::current()
        .timezone()
        .and_then(|name| time.local(&Tz::from_runtime(name)))
        .unwrap_or_else(|| time.utc())
        .seconds_since_midnight() as f32;
    let second_angle = secs / 60.0 * TAU;
    let minute_angle = secs / 3_600.0 * TAU;
    let hour_angle = secs / 43_200.0 * TAU;

    col(
        props!(gap: 8.0, cross_align: CrossAlign::Center),
        [
            clock_canvas(size, hour_angle, minute_angle, second_angle),
            text("Clock", style!(size: label_size, color: GRAY_40)),
        ],
    )
}

/// The diameter this layout's proportions are authored against.
const ROUND_NATIVE: f32 = 480.0;

/// Margin from the circle, as a fraction of the diameter.
///
/// The largest square a circle contains would need `(1 − 1/√2) / 2` ≈ 0.146,
/// which is what a layout has to respect when any row might be full width.
/// This column centres its rows and keeps the outermost ones — the LED and
/// icon strips — narrower than the middle, so the corners the square was
/// protecting are never occupied and the extra width is free.
const ROUND_INSET: f32 = 0.10;

/// A round BFM100 has half again the area of the Deck's Small slot, so it
/// carries the fuller set: the animation and clock side by side across the
/// widest part of the circle, then every counter button, the icon strip and
/// the LED effects.
///
/// Geometry scales with the diameter; type does not, per project convention.
fn round_showcase(time: &SystemTime, counts: [u32; 4], diameter: f32) -> Node {
    let scale = diameter / ROUND_NATIVE;

    col(
        props!(
            background: BG_COLOR,
            padding: diameter * ROUND_INSET,
            gap: 8.0 * scale,
            cross_align: CrossAlign::Center
        ),
        [
            row(
                props!(gap: 16.0 * scale, cross_align: CrossAlign::Center),
                [
                    compact_animation(52.0 * scale, "Pulse + Spin", 11),
                    compact_clock(time, 112.0 * scale, 11),
                ],
            ),
            large_controls_section(counts),
            medium_icon_showcase_section(),
            small_led_effects_section(),
            about_modal(),
        ],
    )
}

fn small_compact(time: &SystemTime, counts: [u32; 4]) -> Node {
    col(
        props!(background: BG_COLOR, padding: 6.0, gap: 5.0),
        [
            row(
                props!(gap: 5.0, flex: 1.0),
                [
                    small_panel(pulse_canvas(54.0)),
                    small_panel(small_clock(time, 72.0)),
                ],
            ),
            small_controls_section(counts),
            small_led_effects_section(),
            about_modal(),
        ],
    )
}

fn medium_compact(time: &SystemTime, counts: [u32; 4]) -> Node {
    col(
        props!(background: BG_COLOR, padding: 10.0, gap: 8.0),
        [
            row(
                props!(gap: 12.0, flex: 1.0, cross_align: CrossAlign::Center),
                [
                    compact_animation(68.0, "Pulse + Spin + Color", 11),
                    medium_controls_section(counts),
                    compact_clock(time, 84.0, 11),
                ],
            ),
            medium_led_effects_section(),
            medium_icon_showcase_section(),
            about_modal(),
        ],
    )
}

fn large_panel(child: Node) -> Node {
    center(
        props!(flex: 1.0, background: GRAY_90.with_alpha(0.45), padding: 4.0),
        [child],
    )
}

fn large_compact(time: &SystemTime, counts: [u32; 4]) -> Node {
    col(
        props!(background: BG_COLOR, padding: 6.0, gap: 5.0),
        [
            row(
                props!(gap: 5.0, flex: 1.0),
                [
                    large_panel(compact_animation(54.0, "Pulse + Spin + Color", 11)),
                    large_panel(compact_clock(time, 72.0, 11)),
                ],
            ),
            large_controls_section(counts),
            large_colors_and_icons_section(),
            large_icon_showcase_section(),
            large_led_effects_section(),
            large_rich_text_section(),
            about_modal(),
        ],
    )
}

fn full_showcase(time: &SystemTime, counts: [u32; 4]) -> Node {
    col(
        props!(background: BG_COLOR, padding: 8.0, gap: 8.0),
        [
            row(
                props!(gap: 6.0),
                [
                    buttons_section(counts),
                    animations_section(time),
                    icons_section(),
                    colors_section(),
                ],
            ),
            row(
                props!(gap: 6.0),
                [icon_buttons_section(), rich_text_section(), led_section()],
            ),
            about_modal(),
        ],
    )
}

fn build_ui(size: WidgetSize, shape: ViewportShape, time: &SystemTime, counts: [u32; 4]) -> Node {
    match hello_layout(size.variant, shape) {
        HelloLayout::SmallCompact => small_compact(time, counts),
        HelloLayout::MediumCompact => medium_compact(time, counts),
        HelloLayout::LargeCompact => large_compact(time, counts),
        HelloLayout::FullShowcase => full_showcase(time, counts),
        #[expect(
            clippy::cast_precision_loss,
            reason = "a viewport is a few hundred pixels"
        )]
        HelloLayout::RoundShowcase => {
            round_showcase(time, counts, size.width.min(size.height) as f32)
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn handle_led_clicks(result: &TreeRenderResult) {
    if result.clicks.contains_key("led_solid") {
        led::set_effect(LedEffect::Solid, Color::from_rgb(255, 40, 40), 0, None);
    }
    if result.clicks.contains_key("led_breathe") {
        led::set_effect(
            LedEffect::Breathe,
            Color::from_rgb(40, 255, 40),
            4_000,
            None,
        );
    }
    if result.clicks.contains_key("led_chase") {
        led::set_effect(LedEffect::Chase, Color::from_rgb(40, 40, 255), 1_000, None);
    }
    if result.clicks.contains_key("led_knight") {
        led::set_effect(
            LedEffect::KnightRider,
            Color::from_rgb(255, 165, 0),
            2_000,
            None,
        );
    }
    if result.clicks.contains_key("led_snake") {
        led::set_effect(LedEffect::Snake, Color::from_rgb(0, 255, 200), 1_500, None);
    }
    if result.clicks.contains_key("led_off") {
        led::stop();
    }
}

fn about_modal() -> Node {
    modal(
        "about",
        MODAL_OPEN.get(),
        "About This Demo",
        vec![
            text(
                "This is a showcase of the WASM widget SDK capabilities.",
                style!(size: 14, line_height: 1.5),
            ),
            text(
                "Features demonstrated:",
                style!(size: 14, weight: FontWeight::BOLD),
            ),
            text(
                "\u{2022} Button styles (Primary, Secondary, Tertiary, Danger)",
                style!(size: 14),
            ),
            text(
                "\u{2022} Icon buttons (icon-only, icon+text, built-in)",
                style!(size: 14),
            ),
            text(
                "\u{2022} Animations (rotation, pulse, fade)",
                style!(size: 14),
            ),
            text(
                "\u{2022} Color palette (brand colors, grays)",
                style!(size: 14),
            ),
            text(
                "\u{2022} Rich text (bold, italic, underline, colors)",
                style!(size: 14),
            ),
            text(
                "\u{2022} Modal dialogs with scroll support",
                style!(size: 14),
            ),
            spacer(1.0),
            text(
                "Scroll Test Content",
                style!(size: 16, weight: FontWeight::BOLD, color: VIOLET_50),
            ),
            text(
                "The following paragraphs test scrolling.",
                style!(size: 14, line_height: 1.4),
            ),
            text(
                "Lorem ipsum dolor sit amet, consectetur adipiscing elit.",
                style!(size: 14, line_height: 1.4),
            ),
            text(
                "Ut enim ad minim veniam, quis nostrud exercitation.",
                style!(size: 14, line_height: 1.4),
            ),
            text(
                "Duis aute irure dolor in reprehenderit in voluptate.",
                style!(size: 14, line_height: 1.4),
            ),
            spacer(1.0),
            text(
                "\u{2014} End of content \u{2014}",
                style!(size: 12, color: GRAY_50),
            ),
        ],
        Some(ModalProps {
            height: 600.0,
            ..ModalProps::default()
        }),
    )
}

#[cfg(target_arch = "wasm32")]
fn handle_clicks(result: &bmc_wasm_sdk::TreeRenderResult) {
    let counter_buttons = ["primary", "secondary", "tertiary", "danger"];
    for (i, id) in counter_buttons.iter().enumerate() {
        if result.clicks.contains_key(*id) {
            COUNTS.with(|c| {
                let mut counts = c.borrow_mut();
                counts[i] = counts[i].saturating_add(1);
            });
        }
    }
    if result.clicks.contains_key("open_modal") {
        MODAL_OPEN.set(true);
    }
    if result.clicks.contains_key("about::close") {
        MODAL_OPEN.set(false);
    }

    handle_led_clicks(result);

    request_frame_after(1_000);
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Re-render in response to touch — the host no longer renders on touch by
/// itself, so an interactive widget must ask for the frame here.
#[unsafe(no_mangle)]
#[cfg(target_arch = "wasm32")]
pub extern "C" fn on_touch() {
    request_frame();
}

#[unsafe(no_mangle)]
#[cfg(target_arch = "wasm32")]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetSize {
        width: w,
        height: h,
        variant,
    } = widget_size();
    let size = WidgetSize {
        variant,
        width: w,
        height: h,
    };
    let time = SystemTime::now();
    let counts = COUNTS.with(|c| *c.borrow());
    let shape = widget_viewport().shape;

    let result = render_ui(w, h, build_ui(size, shape, &time, counts));

    handle_clicks(&result);
}

#[cfg(test)]
mod tests {
    use super::{CompactControls, HelloLayout, compact_controls, hello_layout};
    use bmc_wasm_sdk::{SizeVariant, ViewportShape};

    const RECT: ViewportShape = ViewportShape::Rectangular;

    #[test]
    fn compact_variants_drop_full_showcase_content_that_would_overflow() {
        assert_eq!(
            hello_layout(SizeVariant::Small, RECT),
            HelloLayout::SmallCompact
        );
        assert_eq!(
            hello_layout(SizeVariant::Medium, RECT),
            HelloLayout::MediumCompact
        );
        assert_eq!(
            hello_layout(SizeVariant::Large, RECT),
            HelloLayout::LargeCompact
        );
        assert_eq!(
            hello_layout(SizeVariant::Full, RECT),
            HelloLayout::FullShowcase
        );
    }

    #[test]
    fn a_round_display_takes_the_round_layout_whatever_it_measures() {
        // BFM100 is 480×480, which `SizeVariant` reports as one of its
        // rectangular slots; the shape has to win or the layout would be
        // laid out for a slot and cropped by the circle.
        for variant in [
            SizeVariant::Small,
            SizeVariant::Medium,
            SizeVariant::Large,
            SizeVariant::Full,
        ] {
            assert_eq!(
                hello_layout(variant, ViewportShape::Round),
                HelloLayout::RoundShowcase,
                "{variant:?} on a round display"
            );
        }
    }

    #[test]
    fn large_compact_layout_uses_buttons_modal_colors_icons_led_effects_and_rich_text() {
        assert_eq!(
            compact_controls(HelloLayout::SmallCompact),
            CompactControls::SmallAnimationClockModalAndSubsetLedEffects
        );
        assert_eq!(
            compact_controls(HelloLayout::MediumCompact),
            CompactControls::MediumButtonsModalLedEffectsIconShowcaseAndAnimatedText
        );
        assert_eq!(
            compact_controls(HelloLayout::LargeCompact),
            CompactControls::LargeButtonsModalColorsIconsLedEffectsAndRichText
        );
    }
}
