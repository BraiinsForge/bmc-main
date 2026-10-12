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

use super::{PageBackend, PageCreateFailed, PageFaultKind};

pub(crate) struct MockPage {
    pub size_px: usize,
    pub pixels: Vec<u8>,
}

/// In-memory stand-in for the femtovg pages, recording every call so tests
/// can assert on upload rects and the lifetime page budget, not just texels.
#[derive(Default)]
pub(crate) struct MockBackend {
    pub pages: Vec<MockPage>,
    /// Lifetime count, never decremented — the ≤ 21 page bound is about
    /// textures ever created, not textures currently held.
    pub pages_created: usize,
    pub uploads: Vec<(usize, usize, usize, usize, usize)>,
    pub fail_next_upload: Option<PageFaultKind>,
    pub fail_next_create: bool,
}

impl PageBackend for MockBackend {
    type PageId = usize;

    fn create_page(&mut self, size_px: usize) -> Result<Self::PageId, PageCreateFailed> {
        if self.fail_next_create {
            self.fail_next_create = false;
            return Err(PageCreateFailed);
        }
        self.pages.push(MockPage {
            size_px,
            pixels: vec![0; size_px * size_px],
        });
        self.pages_created += 1;
        Ok(self.pages.len() - 1)
    }

    fn upload(
        &mut self,
        page: Self::PageId,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        pixels: &[u8],
    ) -> Result<(), PageFaultKind> {
        if let Some(kind) = self.fail_next_upload.take() {
            return Err(kind);
        }

        let target = self
            .pages
            .get_mut(page)
            .expect("BUG: upload to unknown page");
        assert_eq!(
            pixels.len(),
            width * height,
            "BUG: coverage size disagrees with upload rect"
        );
        assert!(
            x + width <= target.size_px && y + height <= target.size_px,
            "BUG: upload rect leaves the page"
        );

        for row in 0..height {
            let src = row * width;
            let dst = (y + row) * target.size_px + x;
            target.pixels[dst..dst + width].copy_from_slice(&pixels[src..src + width]);
        }
        self.uploads.push((page, x, y, width, height));
        Ok(())
    }
}
