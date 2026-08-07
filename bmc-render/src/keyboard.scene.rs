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

use std::cell::RefCell;

use bmc_gallery::prelude::*;
use bmc_render_keyboard::layout::{ALL_LAYOUTS, KeyboardLayout};
use bmc_render_keyboard::sound::SilentSink;
use bmc_render_keyboard::theme::{HYPERFUSE_SKIN, LLAMA_SKIN};
use bmc_render_keyboard::{
    EnterBehavior, KeyboardCtx, KeyboardResult, KeyboardState, KeyboardTheme,
};

scene_meta! { title: "Components / Controls / Keyboard" }

static LLAMA_THEME: std::sync::OnceLock<KeyboardTheme> = std::sync::OnceLock::new();
static HYPERFUSE_THEME: std::sync::OnceLock<KeyboardTheme> = std::sync::OnceLock::new();

struct SceneState {
    keyboards: Vec<KeyboardState>,
    audio: RodioSink,
}

thread_local! {
    static STATE: RefCell<SceneState> = RefCell::new(SceneState {
        keyboards: ALL_LAYOUTS.iter().map(|_| KeyboardState::new("", "Wi-Fi Password", "Type here...")).collect(),
        audio: RodioSink::new(),
    });
}

fn keyboard_frame(
    layout: &'static KeyboardLayout,
    index: usize,
    sound: bool,
    theme: &'static KeyboardTheme,
    enter: EnterBehavior,
) -> CustomRenderFn {
    Box::new(move |renderer, interaction, width, height, delta_ms| {
        STATE.with_borrow_mut(|s| {
            s.keyboards[index].enter_behavior = enter;
            let mut silent = SilentSink;
            let audio: &mut dyn bmc_render_keyboard::AudioSink =
                if sound { &mut s.audio } else { &mut silent };
            let mut ctx = KeyboardCtx {
                renderer,
                interaction,
                state: &mut s.keyboards[index],
                audio,
                theme,
                width,
                height,
                delta_ms,
            };
            if !matches!(
                bmc_render_keyboard::render_keyboard(&mut ctx, layout),
                KeyboardResult::Editing
            ) {
                *ctx.state =
                    KeyboardState::new("", "Wi-Fi Password", "Type here...").with_enter(enter);
            }
        });
        // Key presses fade out on the delta this is handed, and the result says
        // what the editor did rather than whether one is still fading.
        true
    })
}

#[scene(default)]
fn layouts(ctx: &mut SceneCtx, ui: &mut Ui) {
    let sound = ctx.toggle("Sound", true);
    let size = ctx.slider("Size", 0.4, 0.0, 1.0, 0.01);

    let theme: &'static KeyboardTheme =
        match ctx.radio("Theme", &["Dark", "Light", "Llama", "Hyperfuse"], 0) {
            1 => &KeyboardTheme::CARBON_LIGHT,
            2 => LLAMA_THEME.get_or_init(|| KeyboardTheme::from_skin(&LLAMA_SKIN)),
            3 => HYPERFUSE_THEME.get_or_init(|| KeyboardTheme::from_skin(&HYPERFUSE_SKIN)),
            _ => &KeyboardTheme::CARBON_DARK,
        };

    let enter = match ctx.radio("Enter", &["Disabled", "Newline", "Confirm"], 0) {
        1 => EnterBehavior::InsertNewline,
        2 => EnterBehavior::Confirm,
        _ => EnterBehavior::Disabled,
    };

    let w = (300.0 + size * (1_280.0 - 300.0)) as u32;
    let h = (150.0 + size * (480.0 - 150.0)) as u32;

    for (i, layout) in ALL_LAYOUTS.iter().enumerate() {
        ui.heading(layout.name);
        ui.label(layout.variant);
        let fired =
            ctx.custom_stage_input(ui, (w, h), keyboard_frame(layout, i, sound, theme, enter));
        for event in &fired.actions {
            if let ActionEvent::Click { key, .. } = event {
                action(key);
            }
        }
    }
}
