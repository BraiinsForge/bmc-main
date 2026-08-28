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

//! The Deck kit: everything scenes need beyond the gallery framework —
//! femtovg rendering of SDK trees into device-sized stages.

mod audio;
mod bridge;
mod images;
mod round;
mod settings;
mod size;

pub use audio::RodioSink;
pub use bmc_render::interaction::ActionEvent;
pub use bridge::{CustomRenderFn, DeckSceneCtx, Fired};
pub use images::seed_image;
pub use settings::system_settings;
pub use size::{
    AUTO_HEIGHT_MAX, DEVICE_HEIGHT, DEVICE_VIEWPORTS, DEVICE_WIDTH, DeckSize, DeviceViewport,
    DivHeight,
};
