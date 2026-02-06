// Copyright (C) 2026  Braiins Systems s.r.o.

//! SDK component showcase - buttons, colors, animations, rich text.

use bmc_wasm_sdk::*;
use std::cell::{Cell, RefCell};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use AnimProperty::*;
use Easing::*;
use LoopMode::*;

thread_local! {
    static WIDTH: Cell<u32> = const { Cell::new(1_280) };
    static HEIGHT: Cell<u32> = const { Cell::new(480) };
    static COUNTS: RefCell<[u32; 4]> = const { RefCell::new([0; 4]) };
    static MODAL_OPEN: Cell<bool> = const { Cell::new(false) };
    // Manual fade: elapsed_ms, total 600ms, ease_out_cubic
    static FADE_ELAPSED: Cell<u32> = const { Cell::new(0) };
}

// Background color (burgundy/magenta)
const BG_COLOR: u32 = 0x66_23_47_FF;
const FADE_DURATION: u32 = 600;

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

#[unsafe(no_mangle)]
pub extern "C" fn init(width: u32, height: u32) {
    WIDTH.set(width);
    HEIGHT.set(height);
}

#[unsafe(no_mangle)]
pub extern "C" fn render(delta_ms: u32) {
    let w = WIDTH.get();
    let h = HEIGHT.get();

    // Manual fade interpolation (no crate needed)
    let elapsed = FADE_ELAPSED.get().saturating_add(delta_ms);
    FADE_ELAPSED.set(elapsed);
    let fade = ease_out_cubic((elapsed as f32 / FADE_DURATION as f32).min(1.0));

    let counts = COUNTS.with(|c| *c.borrow());
    let header_color = color!(GRAY_10, alpha: fade);
    let clock_hour_mark_width = 8.0;
    let clock_hour_mark_height = 1.5;

    let result = render_ui(
        w,
        h,
        col(
            props!(background: BG_COLOR, padding: 24.0, gap: 24.0),
            [
                // Top row: Buttons, Animations, Colors
                row(
                    props!(gap: 32.0),
                    [
                        // Left column: Buttons
                        col(
                            props!(gap: 16.0, flex: 1.0),
                            [
                                text("Buttons", style!(size: 20, color: header_color)),
                                row(
                                    props!(gap: 12.0),
                                    [
                                        button(ButtonStyle::Primary, fmt!("Primary {}", counts[0])),
                                        button(
                                            ButtonStyle::Secondary,
                                            fmt!("Secondary {}", counts[1]),
                                        ),
                                    ],
                                ),
                                row(
                                    props!(gap: 12.0),
                                    [
                                        button(
                                            ButtonStyle::Tertiary,
                                            fmt!("Tertiary {}", counts[2]),
                                        ),
                                        button(ButtonStyle::Danger, fmt!("Danger {}", counts[3])),
                                    ],
                                ),
                                spacer(1.0),
                                button(ButtonStyle::Primary, "Open Modal"),
                            ],
                        ),
                        // Middle column: Animations (declarative)
                        col(
                            props!(gap: 12.0, flex: 1.0),
                            [
                                text("Animations", style!(size: 20, color: header_color)),
                                text("Pulse + Spin", style!(size: 14, color: GRAY_40)),
                                canvas(
                                    props!(width: 64.0, height: 64.0),
                                    [centered(
                                        rect(0.0, 0.0, 32.0, 32.0, VIOLET_50)
                                            .animate(Rotate, 0.0, TAU, 4_000, Linear, Forever)
                                            .animate(Scale, 0.5, 1.0, 1_000, EaseInOut, PingPong),
                                    )],
                                ),
                                text("Clock", style!(size: 14, color: GRAY_40)),
                                canvas(
                                    props!(width: 64.0, height: 64.0, background: GRAY_80),
                                    [
                                        orbit(
                                            26.0,
                                            -PI / 2.0,
                                            rect(
                                                0.0,
                                                0.0,
                                                clock_hour_mark_height,
                                                clock_hour_mark_width,
                                                GRAY_50,
                                            ),
                                        ),
                                        orbit(
                                            26.0,
                                            0.0,
                                            rect(
                                                0.0,
                                                0.0,
                                                clock_hour_mark_width,
                                                clock_hour_mark_height,
                                                GRAY_50,
                                            ),
                                        ),
                                        orbit(
                                            26.0,
                                            PI / 2.0,
                                            rect(
                                                0.0,
                                                0.0,
                                                clock_hour_mark_height,
                                                clock_hour_mark_width,
                                                GRAY_50,
                                            ),
                                        ),
                                        orbit(
                                            26.0,
                                            PI,
                                            rect(
                                                0.0,
                                                0.0,
                                                clock_hour_mark_width,
                                                clock_hour_mark_height,
                                                GRAY_50,
                                            ),
                                        ),
                                        // Orbiting clock hand (declarative)
                                        orbit(18.0, 0.0, rect(0.0, 0.0, 4.0, 4.0, VIOLET_40))
                                            .animate(
                                                OrbitAngle,
                                                -FRAC_PI_2,
                                                3.0 * FRAC_PI_2,
                                                4_000,
                                                Linear,
                                                Forever,
                                            ),
                                        centered(rect(0.0, 0.0, 6.0, 6.0, GRAY_10)),
                                    ],
                                ),
                            ],
                        ),
                        // Right column: Colors
                        col(
                            props!(gap: 12.0, flex: 1.0),
                            [
                                text("Colors", style!(size: 20, color: header_color)),
                                text("Brand", style!(size: 12, color: GRAY_50)),
                                canvas(
                                    props!(width: 180.0, height: 40.0),
                                    [
                                        rect(0.0, 0.0, 36.0, 36.0, VIOLET_50),
                                        rect(44.0, 0.0, 36.0, 36.0, GREEN_50),
                                        rect(88.0, 0.0, 36.0, 36.0, RED_50),
                                        rect(132.0, 0.0, 36.0, 36.0, ORANGE_50),
                                    ],
                                ),
                                text("Grays", style!(size: 12, color: GRAY_50)),
                                canvas(
                                    props!(width: 180.0, height: 28.0),
                                    [
                                        rect(0.0, 0.0, 28.0, 24.0, GRAY_10),
                                        rect(36.0, 0.0, 28.0, 24.0, GRAY_30),
                                        rect(72.0, 0.0, 28.0, 24.0, GRAY_50),
                                        rect(108.0, 0.0, 28.0, 24.0, GRAY_70),
                                        rect(144.0, 0.0, 28.0, 24.0, GRAY_90),
                                    ],
                                ),
                            ],
                        ),
                    ],
                ),
                // Bottom section: Rich text demos
                col(
                    props!(gap: 12.0, background: color!(GRAY_90, alpha: 0.9), padding: 16.0),
                    [
                        text("Rich Text", style!(size: 20, color: header_color)),
                        row(
                            props!(gap: 24.0),
                            [
                                // Left: wrapping paragraph
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
                                                    style!(weight: 700),
                                                ),
                                                span(
                                                    ". Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea ",
                                                    (),
                                                ),
                                                span("commodo consequat", style!(color: VIOLET_50)),
                                                span(
                                                    ". Duis aute irure dolor in reprehenderit.",
                                                    (),
                                                ),
                                            ],
                                        ),
                                    ],
                                ),
                                // Right: style showcase
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
                                                span(
                                                    "colored",
                                                    style!(color: GREEN_50, weight: 700),
                                                ),
                                                span(" text.", ()),
                                            ],
                                        ),
                                    ],
                                ),
                            ],
                        ),
                    ],
                ),
                // Modal overlay (rendered on top when open)
                modal(
                    1, // modal_id
                    MODAL_OPEN.get(),
                    "About This Demo",
                    600.0, // content_height estimate
                    [
                        text(
                            "This is a showcase of the WASM widget SDK capabilities.",
                            style!(size: 14, line_height: 1.5),
                        ),
                        text("Features demonstrated:", style!(size: 14, weight: 700)),
                        text(
                            "• Button styles (Primary, Secondary, Tertiary, Danger)",
                            style!(size: 14),
                        ),
                        text("• Animations (rotation, pulse, fade)", style!(size: 14)),
                        text("• Color palette (brand colors, grays)", style!(size: 14)),
                        text(
                            "• Rich text (bold, italic, underline, colors)",
                            style!(size: 14),
                        ),
                        text("• Modal dialogs with scroll support", style!(size: 14)),
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
                        text("— End of content —", style!(size: 12, color: GRAY_50)),
                    ],
                ),
            ],
        ),
    );

    // Handle button clicks
    // Buttons 0-3: counter buttons (Primary, Secondary, Tertiary, Danger)
    // Button 4: "Open Modal"
    // Button 5: Modal close (only when modal is open)
    for (i, &clicked) in result.clicks.iter().enumerate() {
        if clicked {
            match i {
                0..=3 => {
                    COUNTS.with(|c| {
                        let mut counts = c.borrow_mut();
                        counts[i] = counts[i].saturating_add(1);
                    });
                }
                4 => MODAL_OPEN.set(true),
                5 => MODAL_OPEN.set(false),
                _ => {}
            }
        }
    }

    // Only need to request frame for manual fade — host auto-requests for
    // declarative animations and modal open/close.
    if elapsed < FADE_DURATION {
        request_frame();
    }
}
