// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

/// Custom icon loaded from SVG at compile time via `include_svg!`.
const STAR: Svg = include_svg!("bmc-wasm-runtime/sdk/assets/icons/star.svg");

story_meta! { title: "Button" }

#[story(default)]
fn examples(c: &mut StoryCtx) {
    let label = c.text("Label", "Click me");

    let on_primary_1 = c.action("primary-1");
    let on_secondary_1 = c.action("secondary-1");
    let on_danger_1 = c.action("danger-1");
    let on_tertiary_1 = c.action("tertiary-1");
    let on_ghost_1 = c.action("ghost-1");

    let on_primary_2 = c.action("primary-2");
    let on_secondary_2 = c.action("secondary-2");
    let on_danger_2 = c.action("danger-2");
    let on_tertiary_2 = c.action("tertiary-2");
    let on_ghost_2 = c.action("ghost-2");

    let on_small = c.action("small");
    let on_normal = c.action("normal");
    let on_large = c.action("large");

    c.ui.header("Styles", "");
    c.ui.div(
        (500, AutoH),
        col(
            props!(gap: 24, padding: 16),
            [row(
                props!(gap: 24),
                [
                    col(
                        props!(gap: 12),
                        [
                            button!(&on_primary_1, &label, style: Primary),
                            button!(&on_secondary_1, &label, style: Secondary),
                            button!(&on_danger_1, &label, style: Danger),
                            button!(&on_tertiary_1, &label, style: Tertiary),
                            button!(&on_ghost_1, &label, style: Ghost),
                        ],
                    ),
                    col(
                        props!(gap: 12),
                        [
                            button!(on_primary_2, &label, style: Primary, disabled: true),
                            button!(on_secondary_2, &label, style: Secondary, disabled: true),
                            button!(on_danger_2, &label, style: Danger, disabled: true),
                            button!(on_tertiary_2, &label, style: Tertiary, disabled: true),
                            button!(on_ghost_2, &label, style: Ghost, disabled: true),
                        ],
                    ),
                ],
            )],
        ),
    );
    c.ui.header("Sizes", "");
    c.ui.div(
        (500, AutoH),
        col(
            props!(gap: 12, padding: 16),
            [
                button!(&on_small, &label, size: Small),
                button!(&on_normal, &label, size: Normal),
                button!(&on_large, &label, size: Large),
            ],
        ),
    );
}

#[story]
fn icons(c: &mut StoryCtx) {
    let label = c.text("Label", "Click me");

    let on_icon_1 = c.action("icon-label-1");
    let on_icon_2 = c.action("icon-label-2");
    let on_icon_3 = c.action("icon-label-3");
    let on_icon_4 = c.action("icon-label-4");
    let on_icon_5 = c.action("icon-label-5");

    c.ui.header("With icons", "Label + built-in icon");
    c.ui.div(
        (500, AutoH),
        col(
            props!(gap: 12, padding: 16),
            [
                button!(&on_icon_1, &label, icon: ICON_PLUS, style: Primary),
                button!(&on_icon_2, &label, icon: ICON_CLOSE, style: Secondary),
                button!(&on_icon_3, &label, icon: ICON_WARNING, style: Danger),
                button!(&on_icon_4, &label, icon: ICON_INFO, style: Tertiary),
                button!(&on_icon_5, &label, icon: ICON_SUCCESS, style: Ghost),
            ],
        ),
    );

    let on_only_1 = c.action("icon-only-1");
    let on_only_2 = c.action("icon-only-2");
    let on_only_3 = c.action("icon-only-3");
    let on_only_4 = c.action("icon-only-4");

    c.ui.header("Icon only", "No label");
    c.ui.div(
        (500, AutoH),
        row(
            props!(gap: 12, padding: 16),
            [
                button!(&on_only_1, "", icon: ICON_PLUS, style: Primary),
                button!(&on_only_2, "", icon: ICON_CLOSE, style: Secondary),
                button!(&on_only_3, "", icon: ICON_MINUS, style: Danger),
                button!(&on_only_4, "", icon: ICON_METER, style: Ghost),
            ],
        ),
    );

    let star_id = ensure_registered(&STAR);
    let on_custom_1 = c.action("custom-1");
    let on_custom_2 = c.action("custom-2");

    c.ui.header("Custom icon", "SVG loaded via include_svg!");
    c.ui.div(
        (500, AutoH),
        row(
            props!(gap: 12, padding: 16),
            [
                button!(&on_custom_1, "Favorite", icon: star_id, style: Primary),
                button!(&on_custom_2, "", icon: star_id, style: Secondary),
            ],
        ),
    );
}
