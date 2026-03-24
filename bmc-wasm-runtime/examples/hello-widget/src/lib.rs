// Copyright (C) 2025  Braiins Systems s.r.o.

//! SDK component showcase - buttons, colors, animations, rich text.

use bmc_wasm_sdk::animation::{DynTween, easing};
use bmc_wasm_sdk::*;
use std::cell::{Cell, RefCell};
use std::f32::consts::PI;

thread_local! {
    static WIDTH: Cell<u32> = const { Cell::new(1_280) };
    static HEIGHT: Cell<u32> = const { Cell::new(480) };
    static COUNTS: RefCell<[u32; 4]> = const { RefCell::new([0; 4]) };
    static PULSE_DIR: Cell<bool> = const { Cell::new(true) };
    static MODAL_OPEN: Cell<bool> = const { Cell::new(false) };
    /// Track modal state changes for animation timing
    static MODAL_PREV: Cell<bool> = const { Cell::new(false) };
    /// Frames since modal state changed (for animation duration)
    static MODAL_ANIM_FRAMES: Cell<u32> = const { Cell::new(0) };
}

animated!(FADE: f32);
animated!(ROTATION: f32);
animated!(PULSE: f32);

// Background color (burgundy/magenta)
const BG_COLOR: u32 = 0x66_23_47_FF;

#[unsafe(no_mangle)]
pub extern "C" fn init(width: u32, height: u32) {
    WIDTH.set(width);
    HEIGHT.set(height);

    FADE::start(0.0, 1.0, 600, easing::ease_out_cubic);
    ROTATION::start(0.0, PI * 2.0, 4_000, easing::linear);
    PULSE::start(0.5, 1.0, 1_000, easing::ease_in_out);
}

#[unsafe(no_mangle)]
pub extern "C" fn render(delta_ms: u32) {
    let w = WIDTH.get();
    let h = HEIGHT.get();
    let modal_open = MODAL_OPEN.get();

    // Tick fade animation (always, for initial load)
    FADE::tick(delta_ms);

    // Only tick background animations when modal is closed
    if !modal_open {
        ROTATION::tick(delta_ms);
        PULSE::tick(delta_ms);

        // Loop rotation
        if ROTATION::is_finished() {
            ROTATION::reset();
        }

        // Ping-pong pulse
        if PULSE::is_finished() {
            let growing = PULSE_DIR.get();
            PULSE_DIR.set(!growing);
            PULSE::with(|p| {
                p.set_tween(if growing {
                    DynTween::new(1.0, 0.5, 1_000, easing::ease_in_out)
                } else {
                    DynTween::new(0.5, 1.0, 1_000, easing::ease_in_out)
                });
            });
        }
    }

    let fade = FADE::get();
    let rotation = ROTATION::get();
    let pulse = PULSE::get();

    let counts = COUNTS.with(|c| *c.borrow());
    let header_color = color!(GRAY_10, alpha: fade);
    let pulse_size = 32.0 * pulse;
    let clock_hour_mark_width = 8.0;
    let clock_hour_mark_height = 1.5;

    let _wf = w as f32;
    let _hf = h as f32;

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
                                        button(
                                            ButtonStyle::Primary,
                                            format!("Primary {}", counts[0]),
                                        ),
                                        button(
                                            ButtonStyle::Secondary,
                                            format!("Secondary {}", counts[1]),
                                        ),
                                    ],
                                ),
                                row(
                                    props!(gap: 12.0),
                                    [
                                        button(
                                            ButtonStyle::Tertiary,
                                            format!("Tertiary {}", counts[2]),
                                        ),
                                        button(
                                            ButtonStyle::Danger,
                                            format!("Danger {}", counts[3]),
                                        ),
                                    ],
                                ),
                                spacer(1.0),
                                button(ButtonStyle::Primary, "Open Modal"),
                            ],
                        ),
                        // Middle column: Animations
                        col(
                            props!(gap: 12.0, flex: 1.0),
                            [
                                text("Animations", style!(size: 20, color: header_color)),
                                text("Pulse + Spin", style!(size: 14, color: GRAY_40)),
                                canvas(
                                    props!(width: 64.0, height: 64.0),
                                    [rotated(
                                        rotation,
                                        centered(rect(0.0, 0.0, pulse_size, pulse_size, VIOLET_50)),
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
                                        orbit(
                                            18.0,
                                            rotation - PI / 2.0,
                                            rect(0.0, 0.0, 4.0, 4.0, VIOLET_40),
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
                4 => {
                    // Open modal
                    MODAL_OPEN.set(true);
                }
                5 => {
                    // Close modal
                    MODAL_OPEN.set(false);
                }
                _ => {}
            }
        }
    }

    // Re-read modal state after processing clicks
    let modal_open = MODAL_OPEN.get();

    // Track modal state changes for animation
    let modal_prev = MODAL_PREV.get();
    if modal_open != modal_prev {
        MODAL_PREV.set(modal_open);
        MODAL_ANIM_FRAMES.set(0);
    }
    let modal_anim_frames = MODAL_ANIM_FRAMES.get();
    MODAL_ANIM_FRAMES.set(modal_anim_frames.saturating_add(1));

    // Request next frame only when needed:
    // - Initial fade animation running
    // - Modal closed (background animations running)
    // - Modal animating (first ~20 frames after state change, ~300ms at 60fps)
    let fade_running = !FADE::is_finished();
    let modal_animating = modal_anim_frames < 20;

    if fade_running || !modal_open || modal_animating {
        request_frame();
    }
}
