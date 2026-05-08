// Copyright (C) 2026  Braiins Systems s.r.o.

use std::cell::RefCell;

use crate::prelude::*;
use bmc_render_keyboard::layout::{ALL_LAYOUTS, KeyboardLayout};
use bmc_render_keyboard::sound::SilentSink;
use bmc_render_keyboard::theme::{HYPERFUSE_SKIN, LLAMA_SKIN};
use bmc_render_keyboard::{
    EnterBehavior, KeyboardCtx, KeyboardResult, KeyboardState, KeyboardTheme,
};

story_meta! { title: "Keyboard" }

static LLAMA_THEME: std::sync::OnceLock<KeyboardTheme> = std::sync::OnceLock::new();
static HYPERFUSE_THEME: std::sync::OnceLock<KeyboardTheme> = std::sync::OnceLock::new();

struct StoryState {
    keyboards: Vec<KeyboardState>,
    audio: RodioSink,
}

thread_local! {
    static STATE: RefCell<StoryState> = RefCell::new(StoryState {
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
    })
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "pixel lerp"
)]
#[story(default)]
fn layouts(ctx: &mut StoryCtx) {
    ctx.action_with_key("Cancel", "kb::cancel");
    ctx.action_with_key("Confirm", "kb::confirm");

    let sound = ctx.toggle("Sound", true).get();
    let size = ctx.slider("Size", 0.4, 0.0, 1.0).get();

    let skin_idx = ctx
        .radio("Theme", &["Dark", "Light", "Llama", "Hyperfuse"], 0)
        .get();
    let theme: &'static KeyboardTheme = match skin_idx {
        1 => &KeyboardTheme::CARBON_LIGHT,
        2 => LLAMA_THEME.get_or_init(|| KeyboardTheme::from_skin(&LLAMA_SKIN)),
        3 => HYPERFUSE_THEME.get_or_init(|| KeyboardTheme::from_skin(&HYPERFUSE_SKIN)),
        _ => &KeyboardTheme::CARBON_DARK,
    };

    let enter_idx = ctx
        .radio("Enter", &["Disabled", "Newline", "Confirm"], 0)
        .get();
    let enter = match enter_idx {
        1 => EnterBehavior::InsertNewline,
        2 => EnterBehavior::Confirm,
        _ => EnterBehavior::Disabled,
    };

    let w = (300.0 + size * (1_280.0 - 300.0)) as u32;
    let h = (150.0 + size * (480.0 - 150.0)) as u32;
    let cols = if w > 500 { 2 } else { 3 };

    ctx.ui.grid(cols, 12.0, |grid| {
        for (i, layout) in ALL_LAYOUTS.iter().enumerate() {
            grid.cell(|ui| {
                ui.header(layout.name, layout.variant);
                ui.div_custom((w, h), keyboard_frame(layout, i, sound, theme, enter));
            });
        }
    });
}
