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

use bmc_gallery::prelude::*;

/// Custom icon loaded from SVG at compile time via `include_svg!`.
const STAR: Svg = include_svg!("bmc-wasm-runtime/sdk/assets/icons/star.svg");

scene_meta! { title: "Components / Controls / Button" }

/// Report every tap to the Actions panel by the key that was hit — the buttons
/// here demonstrate styling, so which one fired is the whole story.
fn log_clicks(fired: &Fired) {
    for event in &fired.actions {
        if let ActionEvent::Click { key, .. } = event {
            action(key);
        }
    }
}

#[scene(default)]
fn examples(ctx: &mut SceneCtx, ui: &mut Ui) {
    let label = ctx.text("Label", "Click me");

    ui.heading("Styles");
    let styles = ctx.node_stage_input(ui, (500_usize, AutoH), || {
        col(
            props!(gap: 24, padding: 16),
            [row(
                props!(gap: 24),
                [
                    col(
                        props!(gap: 12),
                        [
                            button!("primary-1", &label, style: Primary),
                            button!("secondary-1", &label, style: Secondary),
                            button!("danger-1", &label, style: Danger),
                            button!("tertiary-1", &label, style: Tertiary),
                            button!("ghost-1", &label, style: Ghost),
                        ],
                    ),
                    col(
                        props!(gap: 12),
                        [
                            button!("primary-2", &label, style: Primary, disabled: true),
                            button!("secondary-2", &label, style: Secondary, disabled: true),
                            button!("danger-2", &label, style: Danger, disabled: true),
                            button!("tertiary-2", &label, style: Tertiary, disabled: true),
                            button!("ghost-2", &label, style: Ghost, disabled: true),
                        ],
                    ),
                ],
            )],
        )
    });
    log_clicks(&styles);

    ui.heading("Sizes");
    let sizes = ctx.node_stage_input(ui, (500_usize, AutoH), || {
        col(
            props!(gap: 12, padding: 16),
            [
                button!("small", &label, size: Small),
                button!("normal", &label, size: Normal),
                button!("large", &label, size: Large),
            ],
        )
    });
    log_clicks(&sizes);
}

#[scene]
fn icons(ctx: &mut SceneCtx, ui: &mut Ui) {
    let label = ctx.text("Label", "Click me");

    ui.heading("With icons");
    ui.label("Label + built-in icon");
    let labelled = ctx.node_stage_input(ui, (500_usize, AutoH), || {
        col(
            props!(gap: 12, padding: 16),
            [
                button!("icon-label-1", &label, icon: ICON_PLUS, style: Primary),
                button!("icon-label-2", &label, icon: ICON_CLOSE, style: Secondary),
                button!("icon-label-3", &label, icon: ICON_WARNING, style: Danger),
                button!("icon-label-4", &label, icon: ICON_INFO, style: Tertiary),
                button!("icon-label-5", &label, icon: ICON_SUCCESS, style: Ghost),
            ],
        )
    });
    log_clicks(&labelled);

    ui.heading("Icon only");
    ui.label("No label");
    let icon_only = ctx.node_stage_input(ui, (500_usize, AutoH), || {
        row(
            props!(gap: 12, padding: 16),
            [
                button!("icon-only-1", "", icon: ICON_PLUS, style: Primary),
                button!("icon-only-2", "", icon: ICON_CLOSE, style: Secondary),
                button!("icon-only-3", "", icon: ICON_MINUS, style: Danger),
                button!("icon-only-4", "", icon: ICON_METER, style: Ghost),
            ],
        )
    });
    log_clicks(&icon_only);

    ui.heading("Custom icon");
    ui.label("SVG loaded via include_svg!");
    let custom = ctx.node_stage_input(ui, (500_usize, AutoH), || {
        // Registered in here, where the registrars are live: a scene rendered
        // before any stage has drawn has nothing to register against.
        let star_id = ensure_registered(&STAR);
        row(
            props!(gap: 12, padding: 16),
            [
                button!("custom-1", "Favorite", icon: star_id, style: Primary),
                button!("custom-2", "", icon: star_id, style: Secondary),
            ],
        )
    });
    log_clicks(&custom);
}
