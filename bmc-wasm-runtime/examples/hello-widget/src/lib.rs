// Copyright (C) 2025  Braiins Systems s.r.o.

//! Simple "Hello World" widget using declarative UI.

use bmc_wasm_sdk::*;
use std::cell::{Cell, RefCell};

thread_local! {
    static WIDTH: Cell<u32> = const { Cell::new(1280) };
    static HEIGHT: Cell<u32> = const { Cell::new(480) };
    static COUNTS: RefCell<[u32; 5]> = const { RefCell::new([0; 5]) };
}

#[unsafe(no_mangle)]
pub extern "C" fn init(width: u32, height: u32) {
    WIDTH.set(width);
    HEIGHT.set(height);
}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let w = WIDTH.get();
    let h = HEIGHT.get();
    let (c0, c1, c2, c3, c4) = COUNTS.with(|counts| {
        let counts = counts.borrow();
        (counts[0], counts[1], counts[2], counts[3], counts[4])
    });

    let clicks = ui::render(
        w,
        h,
        col(
            props!(),
            [
                // Header
                row(
                    props!(padding: 12.0, background: color!(GRAY_100, alpha: 0.4)),
                    [col(
                        props!(gap: 16.0, padding: 16.0),
                        [
                            text("¡Hello from WASM!", 24, props!()),
                            text("Click the buttons below", 14, props!(color: GRAY_30)),
                        ],
                    )],
                ),
                // Content - button row with gap
                center(
                    props!(),
                    [row(
                        props!(gap: 16.0),
                        [
                            button(ButtonStyle::Primary, &format!("Primary: {c0}")),
                            button(ButtonStyle::Secondary, &format!("Secondary: {c1}")),
                            button(ButtonStyle::Tertiary, &format!("Tertiary: {c2}")),
                            button(ButtonStyle::Ghost, &format!("Ghost: {c3}")),
                            button(ButtonStyle::Danger, &format!("Danger: {c4}")),
                        ],
                    )],
                ),
            ],
        ),
    );

    // Handle button clicks
    for (i, &clicked) in clicks.iter().enumerate() {
        if clicked {
            COUNTS.with(|counts| {
                let mut counts = counts.borrow_mut();
                counts[i] = counts[i].saturating_add(1);
            });
        }
    }

    request_frame();
}
