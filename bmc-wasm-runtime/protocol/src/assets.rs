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

pub use bmc_ui::assets::*;

pub const PACKAGE_ASSET_SECTION_NAME: &str = "bmc_assets_v1";
pub const PACKAGE_ASSET_RECORD_MAGIC: [u8; 8] = *b"BMCASSET";
pub const PACKAGE_ASSET_FORMAT_VERSION: u16 = 1;
pub const PACKAGE_ASSET_FORMAT_FLAGS: u8 = 0;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BitmapSampling {
    Linear,
    Nearest,
}
