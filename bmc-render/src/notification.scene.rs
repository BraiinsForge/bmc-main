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

scene_meta! { title: "Components / Feedback / Notification" }

#[scene(default)]
fn notification_kinds(ctx: &mut SceneCtx, ui: &mut Ui) {
    let title = ctx.text("Title", "Something happened");
    let subtitle = ctx.text("Subtitle", "Additional details about this event");

    ctx.node_stage(ui, Page, || {
        col(
            props!(gap: 8, padding: 16, width: 320),
            [
                notification(NotificationKind::Info, &title, &subtitle),
                notification(NotificationKind::Success, &title, &subtitle),
                notification(NotificationKind::Warning, &title, &subtitle),
                notification(NotificationKind::Error, &title, &subtitle),
            ],
        )
    });
}

/// What a widget's surface shows once the host stops it for using too many resources,
/// over a frame the widget can no longer draw, at every widget size, BFM's round face masked.
#[scene]
fn stopped_widget(ctx: &mut SceneCtx, ui: &mut Ui) {
    for viewport in DEVICE_VIEWPORTS {
        ui.heading(viewport.label);
        ctx.custom_stage(
            ui,
            viewport.size,
            Box::new(|renderer, _interaction, _w, _h, _delta| {
                let intl = intl();
                bmc_render::tree::render_stopped_widget_banner(
                    &intl,
                    bmc_render::ui::BLACK,
                    renderer,
                );
                false
            }),
        );
    }
}
