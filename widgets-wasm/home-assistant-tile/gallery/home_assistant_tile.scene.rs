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

//! The tile's building blocks, each on its own at the widths a tile gives it, in every state.

use bmc_gallery::prelude::*;
use home_assistant_tile::view::blocks::badge::{Badge, Disc, Feedback, Mark};
use home_assistant_tile::view::blocks::chips::{Chip, Chips, Role};
use home_assistant_tile::view::blocks::control_button::{ControlButton, Face, Look};
use home_assistant_tile::view::blocks::control_switch::ControlSwitch;
use home_assistant_tile::view::blocks::gradient;
use home_assistant_tile::view::blocks::keypad::{Digit, Key, Keypad, with_dots};
use home_assistant_tile::view::blocks::page_button::PageButton;
use home_assistant_tile::view::blocks::stepper::{Ends, Gradient, Level, Stepper, Touched, Track};
use home_assistant_tile::view::icons::{ARROW_LEFT, ARROW_RIGHT, BACKSPACE_OUTLINE, CHECK, CLOSE};
use home_assistant_tile::view::style::{Accent, CONTROL_HEIGHT, ICON_SIZE, Icon};
use home_assistant_tile::view::touches::Touches;

scene_meta! { title: "Widgets / Home Assistant Tile / Blocks" }

/// Around each block, as a tile's own padding would be.
const PADDING: u32 = 24;
const SMALL_TILE_WIDTH: u32 = 317;
const LARGE_TILE: (u32, u32) = (638, 480);
/// A small tile's row, where buttons and gaps tighten, and a large tile's.
const WIDTHS: [(u32, &str); 2] = [
    (SMALL_TILE_WIDTH - 2 * PADDING, "Small tile"),
    (LARGE_TILE.0 - 2 * PADDING, "Large tile"),
];

const ACCENTS: [(Accent, &str); 9] = [
    (Accent::Amber, "Amber"),
    (Accent::Neutral, "Neutral"),
    (Accent::DeepOrange, "Deep orange"),
    (Accent::Orange, "Orange"),
    (Accent::Blue, "Blue"),
    (Accent::Cyan, "Cyan"),
    (Accent::Green, "Green"),
    (Accent::Purple, "Purple"),
    (Accent::Red, "Red"),
];

const MARKS: [(Option<Mark>, &str); 2] = [(None, "None"), (Some(Mark::Unavailable), "Unavailable")];
const PROGRESS: [(Option<f32>, &str); 4] = [
    (None, "None"),
    (Some(0.0), "Start"),
    (Some(0.5), "Half"),
    (Some(1.0), "Done"),
];

const FACES: [&str; 5] = ["Label", "Large label", "Sign", "Icon", "Long label"];
const LOOKS: [&str; 4] = ["Plain", "Unpicked", "Picked", "Accent"];

const VALUES: [(&str, &str); 3] = [
    ("60%", "Short"),
    ("21.5 °C", "Number"),
    ("Medium high fan speed", "Long"),
];
const SHARES: [(f32, &str); 4] = [
    (0.6, "60 %"),
    (0.0, "Empty"),
    (0.01, "Nearly empty"),
    (1.0, "Full"),
];
const TRACKS: [&str; 4] = ["Level", "Value", "Hue", "Colour temperature"];

const LABELS: [&str; 7] = [
    "Off",
    "Heat",
    "Cool",
    "Auto",
    "Dry",
    "Fan only",
    "Heat/cool",
];
const ICONS: [Icon; 5] = [
    Icon(&ARROW_LEFT),
    Icon(&CHECK),
    Icon(&CLOSE),
    Icon(&BACKSPACE_OUTLINE),
    Icon(&ARROW_RIGHT),
];
const COUNTS: [(usize, &str); 3] = [(3, "3"), (5, "5"), (7, "7")];

/// 2000 to 6500 K in 11 even steps, warm to cool, as Home Assistant spreads a light's range.
const TEMPERATURES: [Color; 11] = [
    Color::from_rgb(255, 137, 14),
    Color::from_rgb(255, 157, 65),
    Color::from_rgb(255, 174, 103),
    Color::from_rgb(255, 188, 132),
    Color::from_rgb(255, 201, 157),
    Color::from_rgb(255, 212, 177),
    Color::from_rgb(255, 222, 195),
    Color::from_rgb(255, 231, 211),
    Color::from_rgb(255, 239, 225),
    Color::from_rgb(255, 247, 238),
    Color::from_rgb(255, 254, 250),
];

/// A large tile's keypad: as wide and high as its content, the line above it a control high,
/// and its four rows sharing what is left under that line and the gaps.
const KEYPAD_WIDTH: u32 = LARGE_TILE.0 - 2 * PADDING;
const KEYPAD_HEIGHT: u32 = LARGE_TILE.1 - 2 * PADDING;
const KEYPAD_GAP: f32 = 12.0;
const KEYPAD_ROW_HEIGHT: f32 = (px(KEYPAD_HEIGHT) - CONTROL_HEIGHT - 4.0 * KEYPAD_GAP) / 4.0;
const STATUS_SIZE: u32 = 22;

fn select<T: Copy>(ctx: &mut SceneCtx, name: &str, options: &[(T, &str)]) -> T {
    let labels: Vec<&str> = options.iter().map(|(_, label)| *label).collect();
    options[ctx.select(name, &labels, 0)].0
}

fn accent(ctx: &mut SceneCtx) -> Color {
    select(ctx, "Accent", &ACCENTS).color()
}

/// The part of a block a "Held" knob puts a finger on, which the block draws pressed.
fn held<T: Copy>(ctx: &mut SceneCtx, parts: &[(Option<T>, &str)]) -> Option<T> {
    select(ctx, "Held", parts)
}

/// `node` on black, padded as a tile would pad it.
fn padded(node: Node) -> Node {
    col(props!(background: BLACK, padding: px(PADDING)), [node])
}

/// `block` at each width, `height` high.
fn stages(ui: &mut Ui, ctx: &mut SceneCtx, height: u32, block: impl Fn(f32) -> Node + Copy) {
    ctx.wrapping_flow(
        ui,
        WIDTHS.map(|(width, label)| {
            titled(
                label,
                (width + 2 * PADDING, height + 2 * PADDING),
                move || padded(block(px(width))),
            )
        }),
    );
}

#[scene(default)]
fn button(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.group("Button");
    let face = ctx.select("Face", &FACES, 0);
    let look = ctx.select("Look", &LOOKS, 0);
    let color = accent(ctx);
    let enabled = ctx.toggle("Enabled", true);
    let pressed = ctx.toggle("Pressed", false);
    let tall = ctx.toggle("Tall", false);

    let height = if tall { 2 } else { 1 } * px_height();
    let look = match LOOKS[look] {
        "Plain" => Look::Plain,
        "Unpicked" => Look::Unpicked,
        "Picked" => Look::Picked,
        _ => Look::Accent(color),
    };
    stages(ui, ctx, height, |width| {
        let face = match FACES[face] {
            "Label" => Face::Text {
                text: "Press".to_owned(),
                size: 28,
            },
            "Large label" => Face::Text {
                text: "Press".to_owned(),
                size: 40,
            },
            "Sign" => Face::Text {
                text: "+".to_owned(),
                size: 32,
            },
            "Icon" => Face::Icon {
                icon: Icon(&CHECK),
                size: ICON_SIZE,
            },
            _ => Face::Fitted {
                text: "Medium high fan speed".to_owned(),
                size: 22,
                line: 28.0,
            },
        };
        let button = ControlButton {
            key: "button".to_owned(),
            face,
            look,
            width,
            height: px(height),
            enabled,
        };
        let touches = if pressed {
            button.hold()
        } else {
            Touches::default()
        };
        button.view(&touches)
    });
}

/// A control's height in whole px, which a stage is measured in.
fn px_height() -> u32 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a control is 64 px high"
    )]
    let height = CONTROL_HEIGHT as u32;
    height
}

#[scene]
fn stepper(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.group("Stepper");
    let arrows = ctx.toggle("Arrows", false);
    let value = select(ctx, "Value", &VALUES);
    let track = ctx.select("Track", &TRACKS, 0);
    let share = select(ctx, "Share", &SHARES);
    let hues = gradient::hues();
    let color = accent(ctx);
    let down = ctx.toggle("Down", true);
    let up = ctx.toggle("Up", true);
    let pending = ctx.toggle("Pending", false);
    let held = held(
        ctx,
        &[
            (None, "None"),
            (Some(Touched::Down), "Down"),
            (Some(Touched::Up), "Up"),
        ],
    );

    stages(ui, ctx, px_height(), |width| {
        let stepper = Stepper {
            key: "stepper".to_owned(),
            ends: if arrows { Ends::Arrows } else { Ends::Signs },
            down,
            up,
            pending,
            track: match TRACKS[track] {
                "Level" => Track::Level {
                    value: value.to_owned(),
                    level: Level { share, color },
                },
                "Value" => Track::Value(value.to_owned()),
                "Hue" => Track::Gradient(Gradient {
                    stops: hues.to_vec(),
                    share,
                }),
                _ => Track::Gradient(Gradient {
                    stops: TEMPERATURES.to_vec(),
                    share,
                }),
            },
            width,
        };
        let touches = held.map_or_else(Touches::default, |part| stepper.hold(part));
        stepper.view(&touches)
    });
}

#[scene]
fn switch(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.group("Switch");
    let on = ctx.toggle("On", true);
    let icons = ctx.toggle("Icons", false);
    let enabled = ctx.toggle("Enabled", true);
    let color = accent(ctx);
    let tall = ctx.toggle("Tall", false);

    let height = if tall { 2 } else { 1 } * px_height();
    stages(ui, ctx, height, |width| {
        ControlSwitch {
            key: "switch".to_owned(),
            on,
            icons: icons.then_some([Icon(&CLOSE), Icon(&CHECK)]),
            enabled,
            color,
            width,
            height: px(height),
        }
        .view()
    });
}

#[scene]
fn chips(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.group("Chips");
    let with_icons = ctx.toggle("Icons", false);
    let count = select(ctx, "Count", &COUNTS);
    let selected = ctx.select("Selected", &["First", "Last", "None"], 0);
    let role = if ctx.toggle("Commands", false) {
        Role::Commands
    } else {
        Role::Options
    };
    let accented = ctx.toggle("Accented", true);
    let color = accent(ctx);
    let dimmed = ctx.toggle("Second dimmed", false);
    let held = held(
        ctx,
        &[(None, "None"), (Some(0), "First"), (Some(2), "Third")],
    );

    let count = if with_icons {
        count.min(ICONS.len())
    } else {
        count
    };
    let chips: Vec<Chip> = (0..count)
        .map(|option| Chip {
            label: LABELS[option].to_owned(),
            icon: with_icons.then(|| ICONS[option]),
            accent: accented.then_some(color),
            enabled: !(dimmed && option == 1),
        })
        .collect();
    let selected = match selected {
        _ if role == Role::Commands => None,
        0 => Some(0),
        1 => Some(count - 1),
        _ => None,
    };
    stages(ui, ctx, px_height(), |width| {
        let chips = Chips {
            key: "chips".to_owned(),
            chips: chips.clone(),
            selected,
            role,
            width,
        };
        let touches = held.map_or_else(Touches::default, |option| chips.hold(option));
        chips.view(&touches)
    });
}

#[scene]
fn page_button(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.group("Page button");
    let count = 2 + ctx.select("Pages", &["2", "3", "4"], 1);
    let page = ctx.select("Page", &["First", "Second", "Last"], 0);
    let pressed = ctx.toggle("Pressed", false);
    let page = match page {
        0 => 0,
        1 => 1,
        _ => count - 1,
    };
    ctx.wrapping_flow(
        ui,
        [(56, "Small tile"), (64, "Larger tiles")].map(|(side, label)| {
            titled(
                label,
                (side + 2 * PADDING, px_height() + 2 * PADDING),
                move || {
                    let button = PageButton {
                        key: "page".to_owned(),
                        icon: Icon(&CHECK),
                        page,
                        count,
                        side: px(side),
                    };
                    let touches = if pressed {
                        button.hold()
                    } else {
                        Touches::default()
                    };
                    padded(button.view(&touches))
                },
            )
        }),
    );
}

const DISCS: [(Disc, &str); 2] = [(Disc::Tinted, "Tinted"), (Disc::Bare, "Bare")];

#[scene]
fn badge(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.group("Badge");
    let color = accent(ctx);
    let pressed = ctx.toggle("Pressed", false);
    let progress = select(ctx, "Hold progress", &PROGRESS);
    let mark = select(ctx, "Mark", &MARKS);
    let disc = select(ctx, "Disc", &DISCS);
    ctx.wrapping_flow(
        ui,
        [(80, "80 px"), (128, "128 px")].map(|(diameter, label)| {
            let badge = Badge {
                disc,
                icon: Icon(&CHECK),
                diameter: px(diameter),
                color,
                feedback: if pressed {
                    Feedback::Pressed
                } else {
                    Feedback::Resting
                },
                progress,
                mark,
            };
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a badge's side, a few dozen px"
            )]
            let side = Badge::side(badge.diameter).ceil() as u32;
            titled(label, (side + 2 * PADDING, side + 2 * PADDING), move || {
                padded(badge.view())
            })
        }),
    );
}

#[scene]
fn keypad(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.group("Keypad");
    let digits = [0, 1, 4, 8][ctx.select("Digits", &["0", "1", "4", "8"], 0)];
    let color = accent(ctx);
    let held = held(
        ctx,
        &[
            (None, "None"),
            (Some(Key::Digit(Digit::ALL[5])), "5"),
            (Some(Key::Ok), "OK"),
            (Some(Key::Cancel), "Cancel"),
        ],
    );
    ctx.wrapping_flow(
        ui,
        [titled(
            "Large tile",
            (KEYPAD_WIDTH + 2 * PADDING, KEYPAD_HEIGHT + 2 * PADDING),
            move || {
                let keypad = Keypad {
                    key: "pad".to_owned(),
                    typed: digits > 0,
                    color,
                    width: px(KEYPAD_WIDTH),
                    row_height: KEYPAD_ROW_HEIGHT,
                };
                let touches = held.map_or_else(Touches::default, |key| keypad.hold(key));
                let status = text(
                    if digits == 0 {
                        "Unlock \u{b7} Enter the code".to_owned()
                    } else {
                        "Unlock \u{b7}".to_owned()
                    },
                    style!(size: STATUS_SIZE, weight: FontWeight::REGULAR, color: GRAY_30),
                );
                col(
                    props!(background: BLACK, padding: px(PADDING), gap: KEYPAD_GAP),
                    [
                        row(
                            props!(gap: 16.0, cross_align: CrossAlign::Center),
                            [
                                keypad.cancel(&touches),
                                with_dots(status, STATUS_SIZE, digits, GRAY_30),
                            ],
                        ),
                        keypad.view(&touches),
                    ],
                )
            },
        )],
    );
}
