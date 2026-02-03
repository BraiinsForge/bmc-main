// Copyright (C) 2025  Braiins Systems s.r.o.

//! SDK component showcase - buttons, colors, animations.

use bmc_wasm_sdk::animation::{easing, DynTween, Transform};
use bmc_wasm_sdk::*;
use std::cell::{Cell, RefCell};
use std::f32::consts::PI;

thread_local! {
    static WIDTH: Cell<u32> = const { Cell::new(1280) };
    static HEIGHT: Cell<u32> = const { Cell::new(480) };
    static COUNTS: RefCell<[u32; 5]> = const { RefCell::new([0; 5]) };

    // Animations
    static FADE_IN: RefCell<DynTween<f32>> = RefCell::new(DynTween::linear(0.0, 1.0, 500));
    static ROTATION: RefCell<DynTween<f32>> = RefCell::new(DynTween::linear(0.0, PI * 2.0, 4000));
    static PULSE: RefCell<DynTween<f32>> = RefCell::new(DynTween::linear(0.5, 1.0, 800));
    static PULSE_DIR: Cell<bool> = const { Cell::new(true) };
}

#[unsafe(no_mangle)]
pub extern "C" fn init(width: u32, height: u32) {
    WIDTH.set(width);
    HEIGHT.set(height);

    FADE_IN.with(|t| {
        *t.borrow_mut() = DynTween::new(0.0, 1.0, 600, easing::ease_out_cubic);
    });
    ROTATION.with(|t| {
        *t.borrow_mut() = DynTween::new(0.0, PI * 2.0, 4000, easing::linear);
    });
    PULSE.with(|t| {
        *t.borrow_mut() = DynTween::new(0.5, 1.0, 1000, easing::ease_in_out);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn render(delta_ms: u32) {
    let w = WIDTH.get();
    let h = HEIGHT.get();

    // Tick animations
    FADE_IN.with(|t| t.borrow_mut().tick(delta_ms));
    ROTATION.with(|t| t.borrow_mut().tick(delta_ms));
    PULSE.with(|t| t.borrow_mut().tick(delta_ms));

    // Loop rotation
    ROTATION.with(|t| {
        if t.borrow().is_finished() {
            t.borrow_mut().reset();
        }
    });

    // Ping-pong pulse
    PULSE.with(|t| {
        if t.borrow().is_finished() {
            let growing = PULSE_DIR.get();
            PULSE_DIR.set(!growing);
            *t.borrow_mut() = if growing {
                DynTween::new(1.0, 0.5, 1000, easing::ease_in_out)
            } else {
                DynTween::new(0.5, 1.0, 1000, easing::ease_in_out)
            };
        }
    });

    let fade = FADE_IN.with(|t| t.borrow().value());
    let rotation = ROTATION.with(|t| t.borrow().value());
    let pulse = PULSE.with(|t| t.borrow().value());

    let counts = COUNTS.with(|c| *c.borrow());
    let header_alpha = (fade * 255.0) as u32;
    let header_color = (GRAY_10 & 0xFFFF_FF00) | header_alpha;

    // Full layout using flex with canvas nodes for custom drawing
    let result = ui::render(
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
                            button(ButtonStyle::Primary, &format!("Primary {}", counts[0])),
                            button(ButtonStyle::Secondary, &format!("Secondary {}", counts[1])),
                        ]),
                        row(props!(gap: 12.0), [
                            button(ButtonStyle::Tertiary, &format!("Tertiary {}", counts[2])),
                            button(ButtonStyle::Ghost, &format!("Ghost {}", counts[3])),
                            button(ButtonStyle::Danger, &format!("Danger {}", counts[4])),
                        ]),
                    ],
                ),
                // Middle column: Animations with canvas areas
                col(
                    props!(gap: 12.0, flex: 1.0),
                    [
                        text("Animations", 20, props!(color: header_color)),
                        text("Pulse (ease_in_out)", 14, props!(color: GRAY_40)),
                        canvas(props!(width: 64.0, height: 64.0)), // canvas 0: pulse
                        text("Rotate (linear)", 14, props!(color: GRAY_40)),
                        canvas(props!(width: 64.0, height: 64.0, background: GRAY_80)), // canvas 1: rotation
                    ],
                ),
                // Right column: Colors with canvas areas
                col(
                    props!(gap: 12.0, flex: 1.0),
                    [
                        text("Colors", 20, props!(color: header_color)),
                        text("Brand", 12, props!(color: GRAY_50)),
                        canvas(props!(width: 180.0, height: 40.0)), // canvas 2: brand colors
                        text("Grays", 12, props!(color: GRAY_50)),
                        canvas(props!(width: 180.0, height: 28.0)), // canvas 3: gray colors
                    ],
                ),
            ],
        ),
    );

    // Draw into canvas areas using computed positions
    if result.canvases.len() >= 4 {
        // Canvas 0: Pulsing square
        let c = result.canvases[0];
        let pulse_size = (32.0 * pulse) as i32;
        let cx = c.x + c.width as i32 / 2;
        let cy = c.y + c.height as i32 / 2;
        fill_rect(cx - pulse_size / 2, cy - pulse_size / 2, pulse_size as u32, pulse_size as u32, VIOLET_50);

        // Canvas 1: Rotating dot (background already drawn by canvas)
        let c = result.canvases[1];
        let cx = c.x as f32 + c.width as f32 / 2.0;
        let cy = c.y as f32 + c.height as f32 / 2.0;
        let orbit = 20.0;
        let transform = Transform::rotate_around((cx, cy), rotation);
        let (dx, dy) = transform.apply_point(cx + orbit, cy);
        fill_rect(dx as i32 - 5, dy as i32 - 5, 10, 10, VIOLET_40);
        fill_rect(cx as i32 - 3, cy as i32 - 3, 6, 6, GRAY_10);

        // Canvas 2: Brand color swatches
        let c = result.canvases[2];
        let swatches = [VIOLET_50, GREEN_50, RED_50, ORANGE_50];
        for (i, color) in swatches.iter().enumerate() {
            fill_rect(c.x + (i as i32 * 44), c.y, 36, 36, *color);
        }

        // Canvas 3: Gray swatches
        let c = result.canvases[3];
        let grays = [GRAY_10, GRAY_30, GRAY_50, GRAY_70, GRAY_90];
        for (i, color) in grays.iter().enumerate() {
            fill_rect(c.x + (i as i32 * 36), c.y, 28, 24, *color);
        }
    }

    // Handle button clicks
    for (i, &clicked) in result.clicks.iter().enumerate() {
        if clicked {
            COUNTS.with(|c| c.borrow_mut()[i] = c.borrow()[i].saturating_add(1));
        }
    }

    request_frame();
}
