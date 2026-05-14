// Copyright (C) 2026  Braiins Systems s.r.o.
#![allow(clippy::cast_precision_loss)]

#[expect(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

use std::cell::{Cell, RefCell};
use std::f32::consts::{FRAC_PI_2, TAU};

use AnimProperty::{Rotate, Scale};
use Easing::{EaseInOut, EaseOut, Linear};
use LoopMode::{Forever, PingPong};

const STAR: Icon = include_icon!("assets/star.svg");
const SETTINGS: Icon = include_icon!("assets/settings.svg");
const CHECKMARK: Icon = include_icon!("assets/checkmark.svg");
const WARNING: Icon = include_icon!("assets/warning.svg");
const SEARCH: Icon = include_icon!("assets/search.svg");

thread_local! {
    static COUNTS: RefCell<[u32; 4]> = const { RefCell::new([0; 4]) };
    static MODAL_OPEN: Cell<bool> = const { Cell::new(false) };
}

const BG_COLOR: Color = Color::from_hex(0x66_23_47);

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
    let secs = time.seconds_since_midnight() as f32;
    let second_angle = secs / 60.0 * TAU;
    let minute_angle = secs / 3_600.0 * TAU;
    let hour_angle = secs / 43_200.0 * TAU;

    col(
        props!(gap: 6.0, flex: 1.0),
        [
            text("Animations", style!(size: 20, color: GRAY_10)),
            text("Pulse + Spin + Color", style!(size: 14, color: GRAY_40)),
            canvas(
                props!(width: 64.0, height: 64.0),
                [Draw::centered(
                    Draw::rect(0.0, 0.0, 32.0, 32.0, RED_50)
                        .animate(Rotate, 0.0, TAU, 4_000, Linear, Forever)
                        .animate(Scale, 0.5, 1.0, 1_000, EaseInOut, PingPong)
                        .animate_color(RED_50, GREEN_50, 3_000, EaseInOut, PingPong),
                )],
            ),
            text("Clock", style!(size: 14, color: GRAY_40)),
            clock_canvas(hour_angle, minute_angle, second_angle),
        ],
    )
}

fn clock_canvas(hour_angle: f32, minute_angle: f32, second_angle: f32) -> Node {
    const S: f32 = 128.0;
    const C: f32 = S / 2.0; // center = 64

    let mut draws: Vec<Draw> = Vec::with_capacity(18);

    // Circle background
    draws.push(Draw::circle(C, C, C, GRAY_80));

    // 12 hour marks
    for i in 0..12 {
        let angle = i as f32 / 12.0 * TAU - FRAC_PI_2;
        draws.push(Draw::orbit(
            54.0,
            angle,
            Draw::rect(0.0, 0.0, 3.0, 3.0, GRAY_50),
        ));
    }

    // Hands: rotated() pivots around canvas center
    // Hour hand
    draws.push(
        Draw::rotated(
            hour_angle,
            Draw::rect(C - 3.0, C - 32.0, 6.0, 36.0, GRAY_10),
        )
        .transition(500, EaseOut),
    );
    // Minute hand
    draws.push(
        Draw::rotated(
            minute_angle,
            Draw::rect(C - 2.0, C - 44.0, 4.0, 48.0, GRAY_30),
        )
        .transition(500, EaseOut),
    );
    // Second hand
    draws.push(
        Draw::rotated(
            second_angle,
            Draw::rect(C - 1.0, C - 52.0, 1.0, 56.0, RED_50),
        )
        .transition(200, EaseOut),
    );
    // Center dot
    draws.push(Draw::centered(Draw::rect(0.0, 0.0, 8.0, 8.0, GRAY_10)));

    canvas(props!(width: S, height: S), draws)
}

fn icons_section() -> Node {
    col(
        props!(gap: 12.0, flex: 1.0),
        [
            text("Icons", style!(size: 20, color: GRAY_10)),
            text("Custom (include_icon!)", style!(size: 12, color: GRAY_50)),
            canvas(
                props!(width: 180.0, height: 40.0),
                [
                    Draw::icon(0.0, 4.0, 32.0, 32.0, &STAR, VIOLET_50),
                    Draw::icon(40.0, 4.0, 32.0, 32.0, &SETTINGS, GREEN_50),
                    Draw::icon(80.0, 4.0, 32.0, 32.0, &CHECKMARK, RED_50),
                    Draw::icon(120.0, 4.0, 32.0, 32.0, &WARNING, ORANGE_50),
                ],
            ),
            text("Built-in (icon_builtin)", style!(size: 12, color: GRAY_50)),
            canvas(
                props!(width: 100.0, height: 40.0),
                [
                    Draw::icon_builtin(0.0, 4.0, 32.0, 32.0, ICON_CLOSE, GRAY_10),
                    Draw::icon_builtin(40.0, 4.0, 32.0, 32.0, ICON_CLOSE, RED_50),
                ],
            ),
            text("Animated", style!(size: 12, color: GRAY_50)),
            canvas(
                props!(width: 64.0, height: 64.0),
                [Draw::centered(
                    Draw::icon(0.0, 0.0, 32.0, 32.0, &STAR, ORANGE_50)
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
                    span("consectetur adipiscing elit", style!(weight: 700)),
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
                    span("bold", style!(weight: 700)),
                    span(", ", ()),
                    span("italic", style!(italic: true)),
                    span(", ", ()),
                    span("underline", style!(underline: true)),
                    span(", ", ()),
                    span("strikethrough", style!(strikethrough: true)),
                    span(", and ", ()),
                    span("colored", style!(color: GREEN_50, weight: 700)),
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
            text("LED Effects", style!(size: 14, weight: 700)),
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

fn handle_led_clicks(result: &TreeRenderResult) {
    if result.clicks.contains_key("led_solid") {
        led::enable();
        led::set_effect(LedEffect::Solid, Color::from_rgb(255, 40, 40), 0, 0);
    }
    if result.clicks.contains_key("led_breathe") {
        led::enable();
        led::set_effect(LedEffect::Breathe, Color::from_rgb(40, 255, 40), 4_000, 0);
    }
    if result.clicks.contains_key("led_chase") {
        led::enable();
        led::set_effect(LedEffect::Chase, Color::from_rgb(40, 40, 255), 1_000, 0);
    }
    if result.clicks.contains_key("led_knight") {
        led::enable();
        led::set_effect(
            LedEffect::KnightRider,
            Color::from_rgb(255, 165, 0),
            2_000,
            0,
        );
    }
    if result.clicks.contains_key("led_snake") {
        led::enable();
        led::set_effect(LedEffect::Snake, Color::from_rgb(0, 255, 200), 1_500, 0);
    }
    if result.clicks.contains_key("led_off") {
        led::disable();
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
            text("Features demonstrated:", style!(size: 14, weight: 700)),
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
                style!(size: 16, weight: 700, color: VIOLET_50),
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

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetSize {
        width: w,
        height: h,
        ..
    } = widget_size();
    let time = SystemTime::now();
    let counts = COUNTS.with(|c| *c.borrow());

    let result = render_ui(
        w,
        h,
        col(
            props!(background: BG_COLOR, padding: 8.0, gap: 8.0),
            [
                row(
                    props!(gap: 6.0),
                    [
                        buttons_section(counts),
                        animations_section(&time),
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
        ),
    );

    handle_clicks(&result);
}
