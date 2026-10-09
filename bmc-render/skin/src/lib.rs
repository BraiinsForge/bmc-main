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

//! Skin and 9-patch registration with the host.
//!
//! The types are [`bmc_ui::skin`]'s; this crate adds the one impure step:
//! turning a compile-time asset into a registered bitmap id
//! through a registrar the host installs with [`init`].

use std::cell::RefCell;

pub use bmc_ui::skin::*;
use bmc_ui::{BitmapId, StaticAssetSource};

type BitmapRegistrar = fn(&str, StaticAssetSource) -> Option<BitmapId>;

thread_local! {
    static BITMAP_REGISTRAR: RefCell<BitmapRegistrar> = const {
        RefCell::new(|_, _| panic!("BUG: skin bitmap registrar not initialized — call bmc_render_skin::init() first"))
    };
}

/// Initialize the skin system with a bitmap registration function. Must be
/// called once before any skin or nine-patch registration.
pub fn init(register_fn: BitmapRegistrar) {
    BITMAP_REGISTRAR.with(|r| *r.borrow_mut() = register_fn);
}

fn register_bitmap(tag: &str, source: StaticAssetSource) -> Option<BitmapId> {
    BITMAP_REGISTRAR.with(|r| r.borrow()(tag, source))
}

/// Register a 9-patch asset with the host and return a `NinePatch`.
///
/// Idempotent: the host dedups by `asset.name`, so a second call returns the
/// cached `BitmapId` without re-decoding/re-uploading.
#[must_use]
pub fn ensure_nine_patch_registered(asset: &NinePatchAsset) -> NinePatch {
    NinePatch {
        bitmap_id: register_bitmap(asset.name, asset.source),
        left: asset.left,
        top: asset.top,
        right: asset.right,
        bottom: asset.bottom,
    }
}

/// A [`Skin`]'s image assets, registered with the host on lookup.
pub trait SkinRegistry {
    /// Look up a 9-patch asset by name and register it with the host if needed.
    ///
    /// Registration is idempotent host-side: the tag combines the skin name and
    /// the asset name so two skins with the same asset name don't alias.
    fn get_nine_patch(&self, name: &str) -> Option<SkinEntry>;

    /// Get the preview thumbnail asset, if this skin includes a `preview.png`.
    fn preview(&self) -> Option<SkinEntry> {
        self.get_nine_patch("preview")
    }
}

impl SkinRegistry for Skin {
    fn get_nine_patch(&self, name: &str) -> Option<SkinEntry> {
        let asset = self.asset(name)?;
        let tag = format!("skin::{}::{}", self.name, asset.name);
        Some(SkinEntry {
            nine_patch: NinePatch {
                bitmap_id: register_bitmap(&tag, asset.source),
                left: asset.left,
                top: asset.top,
                right: asset.right,
                bottom: asset.bottom,
            },
            width: asset.width,
            height: asset.height,
            color: asset.color,
        })
    }
}
