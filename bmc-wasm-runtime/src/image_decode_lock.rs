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

//! Device-wide serialization of image decode working sets.
//!
//! The permit bounds the decode working set — decode intermediates
//! and the scaled RGBA buffer, held until the GPU upload consumes it.
//! The encoded input is copied before the permit and bounded separately:
//! it cannot exceed the guest's linear memory, and each runtime caps
//! its in-flight decodes, so waiting decodes pin a bounded backlog.
//! Cache restores stay outside the permit deliberately —
//! they upload pre-decoded RGBA from a file-backed mmap
//! (reclaimable page cache, no decode work),
//! and a blocking flock on the render thread would stall rendering.

use std::fs::{File, OpenOptions};
use std::path::Path;

use anyhow::{Context, Result};

#[must_use]
#[derive(Debug)]
pub struct ImageDecodePermit {
    file: Option<File>,
}

impl ImageDecodePermit {
    pub(crate) fn acquire(path: Option<&Path>) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self { file: None });
        };
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("open image decode lock {}", path.display()))?;
        tracing::debug!(lock_path = %path.display(), "waiting for image decode permit");
        rustix::io::retry_on_intr(|| {
            rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
        })
        .with_context(|| format!("lock image decode permit {}", path.display()))?;
        tracing::debug!(lock_path = %path.display(), "acquired image decode permit");
        Ok(Self { file: Some(file) })
    }
}

impl Drop for ImageDecodePermit {
    fn drop(&mut self) {
        if let Some(file) = self.file.as_ref()
            && let Err(error) = rustix::fs::flock(file, rustix::fs::FlockOperation::Unlock)
        {
            tracing::warn!(?error, "failed to release image decode permit");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;

    use super::ImageDecodePermit;

    #[test]
    fn permit_blocks_an_independent_open_file() {
        let dir = tempfile::tempdir().expect("BUG: tempdir should be available");
        let path = dir.path().join("image-decode.lock");
        let permit = ImageDecodePermit::acquire(Some(&path))
            .expect("BUG: first image decode permit should succeed");
        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("BUG: contender should open the image decode lock");

        let error = rustix::fs::flock(
            &contender,
            rustix::fs::FlockOperation::NonBlockingLockExclusive,
        )
        .expect_err("independent image decode must wait for the held permit");
        assert_eq!(error, rustix::io::Errno::WOULDBLOCK);

        drop(permit);
        rustix::fs::flock(
            &contender,
            rustix::fs::FlockOperation::NonBlockingLockExclusive,
        )
        .expect("image decode permit should become available after release");
    }
}
