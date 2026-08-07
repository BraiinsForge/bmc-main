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

scene_meta! { title: "Components / Feedback / Modal" }

#[scene(default)]
fn examples(ctx: &mut SceneCtx, ui: &mut Ui) {
    let v = ctx.radio("Variant", &["Basic", "With footer", "Danger"], 0);
    let long_body = ctx.toggle("Long body (scrolling)", false);

    // Realistic prose content (3 paragraphs, repeated) used when the
    // "Long body (scrolling)" toggle is on. Crafted so the scroll
    // fallback is visibly exercised rather than showing identical lines.
    let body = if long_body {
        let para_a = "Long-form modal content lays out at its natural height. When the body's \
                      intrinsic height exceeds the modal viewport, the body must be wrapped in a \
                      scroll container — the modal frame itself stays anchored.";
        let para_b = "Wrapping the body in scroll() pins the modal chrome (title bar, footer) \
                      to the modal edges and lets only the body region scroll. This is the \
                      recommended pattern for confirmation dialogs with terms-of-service text, \
                      release notes, log inspectors, and similar dense content.";
        let para_c = "Without the scroll container, the modal would simply size to fit its \
                      intrinsic height, potentially exceeding the screen. The scroll wrapper \
                      bounds the modal height to a fixed viewport while keeping all content \
                      reachable via vertical pan.";

        let mut prose: Vec<Node> = Vec::new();
        for round in 0..3 {
            for (i, para) in [para_a, para_b, para_c].into_iter().enumerate() {
                prose.push(text(
                    &fmt!("§{}.{}  {}", round + 1, i + 1, para),
                    style!(size: 14, color: GRAY_10, line_height: 1.4),
                ));
            }
        }
        prose
    } else {
        match v {
            0 => vec![text(
                "This is a simple modal with no footer actions.",
                style!(size: 14, color: GRAY_10),
            )],
            1 => vec![text(
                "Are you sure you want to proceed?",
                style!(size: 14, color: GRAY_10),
            )],
            // Danger
            _ => vec![text(
                "This action cannot be undone. All associated data will be permanently removed.",
                style!(size: 14, color: GRAY_10),
            )],
        }
    };

    let (key, title, footer) = match v {
        0 => ("basic", "Basic modal", None),
        1 => (
            "footer",
            "Confirm action",
            Some(ModalFooter {
                primary: ModalAction { label: "Confirm" },
                secondary: Some(ModalAction { label: "Cancel" }),
                danger: false,
            }),
        ),
        // Danger
        _ => (
            "danger",
            "Delete item",
            Some(ModalFooter {
                primary: ModalAction { label: "Delete" },
                secondary: Some(ModalAction { label: "Cancel" }),
                danger: true,
            }),
        ),
    };

    let n = modal(
        key,
        true,
        title,
        body,
        Some(ModalProps {
            max_width: 480,
            footer,
            ..Default::default()
        }),
    );

    let content = col(
        props!(padding: 16),
        [
            text(
                "Background content behind modal",
                style!(size: 14, padding: 16),
            ),
            n,
        ],
    );

    // The same modal at every device size: what fits at 1280×480 has to stay
    // usable down to 160×120.
    for size in [Full, Large, Medium, Small] {
        let fired = ctx.node_stage_input(ui, size, || content.clone());
        for event in &fired.actions {
            if let ActionEvent::Click { key, .. } = event {
                action(key);
            }
        }
    }
}
