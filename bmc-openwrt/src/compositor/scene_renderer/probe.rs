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

//! `BMC_COMPOSE_PROBE`, a diagnostic that isolates parts of a frame's GPU cost.

use std::borrow::Cow;

use anyhow::{Context, Result};
use smithay::{
    backend::allocator::Fourcc,
    backend::renderer::{
        Offscreen,
        gles::{GlesRenderer, GlesTexture},
    },
    reexports::wayland_server::backend::ObjectId,
    utils::{Physical, Rectangle, Size, Transform},
};

/// Which part of the frame's GPU cost a run isolates, set by `BMC_COMPOSE_PROBE`
/// in profiling builds.
///
/// Diagnostic only: every mode but `Normal` shows a wrong picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(feature = "profiling"),
    expect(dead_code, reason = "only profiling builds read a probe")
)]
pub(super) enum ComposeProbe {
    Normal,
    /// No clear and no draws, leaving the fixed cost of a frame.
    NoDraw,
    /// One whole-output clear and nothing else.
    ClearOnly,
    /// A clear of the output's top half, to see whether clear cost scales with area.
    HalfClear,
    /// A clear of one small square, to tell per-pixel cost from a whole-surface one.
    TinyClear,
    /// The normal frame with widgets sampled unrotated into the same destinations,
    /// to see what the scanout rotation costs the texture fetch.
    NoRotate,
    /// The normal frame, ignoring new commits to buffers already imported,
    /// to see what importing a client buffer costs the GPU.
    NoImport,
    /// The normal frame drawn into a tiled offscreen texture instead of the linear scanout buffer,
    /// which keeps showing the last frame, to see what linear writes cost.
    Offscreen,
    /// The normal frame over a whole-output clear,
    /// to see whether one is cheaper than the scissored clears.
    FullClear,
}

impl ComposeProbe {
    #[cfg(feature = "profiling")]
    fn from_env() -> Self {
        let probe = match std::env::var("BMC_COMPOSE_PROBE").as_deref() {
            Ok("nodraw") => Self::NoDraw,
            Ok("clearonly") => Self::ClearOnly,
            Ok("halfclear") => Self::HalfClear,
            Ok("tinyclear") => Self::TinyClear,
            Ok("norotate") => Self::NoRotate,
            Ok("noimport") => Self::NoImport,
            Ok("offscreen") => Self::Offscreen,
            Ok("fullclear") => Self::FullClear,
            Ok(other) => {
                tracing::warn!("unknown BMC_COMPOSE_PROBE {other:?}, drawing normally");
                Self::Normal
            }
            Err(_) => Self::Normal,
        };
        if probe != Self::Normal {
            tracing::warn!("compose probe {probe:?}: frames are deliberately wrong");
        }
        probe
    }

    #[cfg(not(feature = "profiling"))]
    fn from_env() -> Self {
        Self::Normal
    }

    pub(super) fn is_normal(self) -> bool {
        self == Self::Normal
    }

    /// The buffers to import this frame out of the `dirty` ones.
    pub(super) fn imports(
        self,
        dirty: &[ObjectId],
        imported: impl Fn(&ObjectId) -> bool,
    ) -> Cow<'_, [ObjectId]> {
        if self == Self::NoImport {
            Cow::Owned(dirty.iter().filter(|id| !imported(id)).cloned().collect())
        } else {
            Cow::Borrowed(dirty)
        }
    }

    /// The regions to clear, `uncovered` giving the ones a normal frame clears.
    pub(super) fn clear_regions(
        self,
        output: Rectangle<i32, Physical>,
        uncovered: impl FnOnce() -> Vec<Rectangle<i32, Physical>>,
    ) -> Vec<Rectangle<i32, Physical>> {
        match self {
            Self::Normal | Self::NoRotate | Self::NoImport | Self::Offscreen => uncovered(),
            Self::NoDraw => Vec::new(),
            Self::ClearOnly | Self::FullClear => vec![output],
            Self::HalfClear => vec![Rectangle::from_size(Size::from((
                output.size.w,
                output.size.h.div_euclid(2),
            )))],
            Self::TinyClear => vec![Rectangle::from_size(Size::from((64, 64)))],
        }
    }

    pub(super) fn draws_content(self) -> bool {
        matches!(
            self,
            Self::Normal | Self::FullClear | Self::NoRotate | Self::NoImport | Self::Offscreen
        )
    }

    /// The transform a widget is drawn with, `scanout` normally.
    pub(super) fn widget_transform(self, scanout: Transform) -> Transform {
        if self == Self::NoRotate {
            Transform::Normal
        } else {
            scanout
        }
    }
}

/// The probe a compositor runs, with the offscreen target [`ComposeProbe::Offscreen`] draws into.
pub(super) struct Probe {
    pub(super) mode: ComposeProbe,
    offscreen: Option<GlesTexture>,
}

impl Probe {
    pub(super) fn from_env() -> Self {
        Self {
            mode: ComposeProbe::from_env(),
            offscreen: None,
        }
    }

    /// The target to draw into instead of the scanout buffer, allocated on its first frame.
    pub(super) fn offscreen_target(
        &mut self,
        renderer: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Result<Option<&mut GlesTexture>> {
        if self.mode != ComposeProbe::Offscreen {
            return Ok(None);
        }
        if self.offscreen.is_none() {
            self.offscreen = Some(
                Offscreen::<GlesTexture>::create_buffer(
                    renderer,
                    Fourcc::Abgr8888,
                    Size::from((size.w, size.h)),
                )
                .context("Failed to allocate the probe's offscreen target")?,
            );
        }
        Ok(self.offscreen.as_mut())
    }
}

#[cfg(test)]
mod tests {
    use smithay::{
        reexports::wayland_server::backend::ObjectId,
        utils::{Physical, Rectangle, Size, Transform},
    };

    use super::ComposeProbe;

    #[test]
    fn a_normal_frame_imports_every_dirty_buffer() {
        let dirty = [ObjectId::null()];

        assert_eq!(&*ComposeProbe::Normal.imports(&dirty, |_| true), &dirty);
    }

    #[test]
    fn a_normal_frame_clears_what_its_widgets_leave_uncovered() {
        let output = Rectangle::<i32, Physical>::from_size(Size::from((10, 10)));
        let uncovered = vec![Rectangle::from_size(Size::from((4, 4)))];

        assert_eq!(
            ComposeProbe::Normal.clear_regions(output, || uncovered.clone()),
            uncovered
        );
    }

    #[test]
    fn a_normal_frame_draws_its_widgets_with_their_scanout_transform() {
        assert_eq!(
            (
                ComposeProbe::Normal.draws_content(),
                ComposeProbe::Normal.widget_transform(Transform::Flipped270)
            ),
            (true, Transform::Flipped270)
        );
    }
}
