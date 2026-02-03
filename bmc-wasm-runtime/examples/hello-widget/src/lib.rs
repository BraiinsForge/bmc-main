// Copyright (C) 2025  Braiins Systems s.r.o.

//! SDK component showcase - buttons, colors, animations.

use bmc_wasm_sdk::*;
use bmc_wasm_sdk::animation::{easing, DynTween};
use std::cell::{Cell, RefCell};
use std::f32::consts::PI;

thread_local! {
    static WIDTH: Cell<u32> = const { Cell::new(1_280) };
    static HEIGHT: Cell<u32> = const { Cell::new(480) };
    static COUNTS: RefCell<[u32; 5]> = const { RefCell::new([0; 5]) };
    static PULSE_DIR: Cell<bool> = const { Cell::new(true) };
}

animated!(FADE: f32);
animated!(ROTATION: f32);
animated!(PULSE: f32);

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

    // Tick animations
    FADE::tick(delta_ms);
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

    let fade = FADE::get();
    let rotation = ROTATION::get();
    let pulse = PULSE::get();

    let counts = COUNTS.with(|c| *c.borrow());
    let header_color = color!(GRAY_10, alpha: fade);
    let pulse_size = 32.0 * pulse;
    let clock_hour_mark_width = 8.0;
    let clock_hour_mark_height = 1.5;

    let result = render_ui(
        w,
        h,
        row(
            props!(padding: 24.0, gap: 32.0),
            [
                // Left column: Buttons
                col(
                    props!(gap: 16.0, flex: 1.0),
                    [
                        text("Buttons", 20, props!(color: header_color)),
                        row(props!(gap: 12.0), [
                            button(ButtonStyle::Primary, format!("Primary {}", counts[0])),
                            button(ButtonStyle::Secondary, format!("Secondary {}", counts[1])),
                        ]),
                        row(props!(gap: 12.0), [
                            button(ButtonStyle::Tertiary, format!("Tertiary {}", counts[2])),
                            button(ButtonStyle::Ghost, format!("Ghost {}", counts[3])),
                            button(ButtonStyle::Danger, format!("Danger {}", counts[4])),
                        ]),
                    ],
                ),
                // Middle column: Animations with canvas areas
                col(
                    props!(gap: 12.0, flex: 1.0),
                    [
                        text("Animations", 20, props!(color: header_color)),
                        text("Pulse + Spin", 14, props!(color: GRAY_40)),
                        canvas(props!(width: 64.0, height: 64.0), [
                            rotated(rotation, centered(rect(0.0, 0.0, pulse_size, pulse_size, VIOLET_50))),
                        ]),
                        text("Clock", 14, props!(color: GRAY_40)),
                        canvas(props!(width: 64.0, height: 64.0, background: GRAY_80), [
                            // Hour marks at 12, 3, 6, 9
                            orbit(26.0, -PI / 2.0, rect(0.0, 0.0, clock_hour_mark_height, clock_hour_mark_width, GRAY_50)),  // 12
                            orbit(26.0, 0.0, rect(0.0, 0.0, clock_hour_mark_width, clock_hour_mark_height, GRAY_50)),        // 3
                            orbit(26.0, PI / 2.0, rect(0.0, 0.0, clock_hour_mark_height, clock_hour_mark_width, GRAY_50)),   // 6
                            orbit(26.0, PI, rect(0.0, 0.0, clock_hour_mark_width, clock_hour_mark_height, GRAY_50)),         // 9
                            // Second hand
                            orbit(18.0, rotation - PI / 2.0, rect(0.0, 0.0, 4.0, 4.0, VIOLET_40)),
                            centered(rect(0.0, 0.0, 6.0, 6.0, GRAY_10)),
                        ]),
                    ],
                ),
                // Right column: Colors with canvas areas
                col(
                    props!(gap: 12.0, flex: 1.0),
                    [
                        text("Colors", 20, props!(color: header_color)),
                        text("Brand", 12, props!(color: GRAY_50)),
                        canvas(props!(width: 180.0, height: 40.0), [
                            rect(0.0, 0.0, 36.0, 36.0, VIOLET_50),
                            rect(44.0, 0.0, 36.0, 36.0, GREEN_50),
                            rect(88.0, 0.0, 36.0, 36.0, RED_50),
                            rect(132.0, 0.0, 36.0, 36.0, ORANGE_50),
                        ]),
                        text("Grays", 12, props!(color: GRAY_50)),
                        canvas(props!(width: 180.0, height: 28.0), [
                            rect(0.0, 0.0, 28.0, 24.0, GRAY_10),
                            rect(36.0, 0.0, 28.0, 24.0, GRAY_30),
                            rect(72.0, 0.0, 28.0, 24.0, GRAY_50),
                            rect(108.0, 0.0, 28.0, 24.0, GRAY_70),
                            rect(144.0, 0.0, 28.0, 24.0, GRAY_90),
                        ]),
                    ],
                ),
            ],
        ),
    );

    // Handle button clicks
    for (i, &clicked) in result.clicks.iter().enumerate() {
        if clicked {
            COUNTS.with(|c| {
                let mut counts = c.borrow_mut();
                counts[i] = counts[i].saturating_add(1);
            });
        }
    }

    request_frame();
}
