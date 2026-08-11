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

// Mouse-to-touch input handling.
// Transforms screen coordinates (1280×480 landscape) to guest coordinates
// and sends InputEvents via the IPC endpoint.

use bmc_virt_ipc::{FB_HEIGHT, FB_WIDTH, HostEndpoint, InputEvent};

/// Handles mouse input over the framebuffer and forwards as touch events.
pub struct InputHandler {
    pressed: bool,
}

impl InputHandler {
    pub fn new() -> Self {
        Self { pressed: false }
    }

    /// Process egui input over the framebuffer display rect.
    /// `screen_rect` is the on-screen rect where the (rotated) framebuffer is displayed.
    /// Returns true if the cursor is hovering over the screen area (for custom cursor).
    pub fn process(
        &mut self,
        ui: &mut egui::Ui,
        screen_rect: egui::Rect,
        ipc: &mut HostEndpoint,
    ) -> bool {
        let response = ui.allocate_rect(screen_rect, egui::Sense::click_and_drag());
        let hovering = response.hovered();
        // Track "button currently held on this widget" separately from
        // `self.pressed`: the latter is only set inside the drag
        // branches, so a pure click would otherwise never flash the
        // cursor indicator. `is_pointer_button_down_on` flips true for
        // every press (click and drag alike) and back to false on up.
        let holding = response.is_pointer_button_down_on();

        // Quick tap: egui reports as click (no drag threshold crossed)
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (gx, gy) = screen_to_guest(pos, screen_rect);
            send(ipc, InputEvent::TouchDown { x: gx, y: gy });
            send(ipc, InputEvent::TouchUp);
        }

        // Sustained press / drag
        if response.drag_started()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (gx, gy) = screen_to_guest(pos, screen_rect);
            self.pressed = true;
            send(ipc, InputEvent::TouchDown { x: gx, y: gy });
        } else if response.dragged()
            && self.pressed
            && let Some(pos) = response.interact_pointer_pos()
        {
            let (gx, gy) = screen_to_guest(pos, screen_rect);
            send(ipc, InputEvent::TouchMove { x: gx, y: gy });
        }

        if response.drag_stopped() && self.pressed {
            send(ipc, InputEvent::TouchUp);
            self.pressed = false;
        }

        // Draw custom cursor when hovering
        if hovering {
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
            if let Some(pos) = ui.ctx().pointer_hover_pos() {
                let painter = ui.painter();
                if holding {
                    // Pressed: larger, more visible
                    painter.circle_filled(pos, 14.0, egui::Color32::from_white_alpha(60));
                    painter.circle_stroke(
                        pos,
                        16.0,
                        egui::Stroke::new(2.0_f32, egui::Color32::from_white_alpha(180)),
                    );
                } else {
                    // Hovering: subtle indicator
                    painter.circle_filled(pos, 10.0, egui::Color32::from_white_alpha(40));
                    painter.circle_stroke(
                        pos,
                        12.0,
                        egui::Stroke::new(1.5_f32, egui::Color32::from_white_alpha(140)),
                    );
                }
            }
        }

        hovering
    }
}

/// Transform screen coordinates to guest touch coordinates.
///
/// The app reads ABS_X/ABS_Y as landscape coordinates (1280×480) — same
/// as what VNC sends. The screen display is also 1280×480, so it's a
/// direct proportional mapping, no rotation needed.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "coordinate math on small positive pixel values"
)]
fn screen_to_guest(screen_pos: egui::Pos2, screen_rect: egui::Rect) -> (u16, u16) {
    let x_frac = ((screen_pos.x - screen_rect.min.x) / screen_rect.width()).clamp(0.0, 1.0);
    let y_frac = ((screen_pos.y - screen_rect.min.y) / screen_rect.height()).clamp(0.0, 1.0);

    // Direct mapping: screen landscape → guest landscape (1280×480)
    let guest_x = (x_frac * FB_HEIGHT as f32) as u16;
    let guest_y = (y_frac * FB_WIDTH as f32) as u16;

    (
        guest_x.min(FB_HEIGHT as u16 - 1),
        guest_y.min(FB_WIDTH as u16 - 1),
    )
}

fn send(ipc: &mut HostEndpoint, event: InputEvent) {
    if let Err(e) = ipc.send_input(event) {
        tracing::warn!("failed to send input: {e}");
    }
}
