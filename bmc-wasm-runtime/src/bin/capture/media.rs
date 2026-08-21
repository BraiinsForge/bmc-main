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

//! Where a rendered artifact lands under a widget's output directory:
//! `<kind>/<platform>-<viewport>/<dataset>.<ext>`.
//!
//! Kind first, then target, so every level answers one question
//! and `current/` cannot be read as a platform.

use std::path::{Path, PathBuf};

/// What a rendered artifact shows, which is the first thing its path says.
#[derive(Clone, Copy)]
pub enum Media {
    /// A baseline beside the diff that failed against it.
    Comparison,
    Preview,
}

impl Media {
    pub fn dir(self) -> &'static str {
        match self {
            Self::Comparison => "comparison",
            Self::Preview => "preview",
        }
    }

    pub fn path(self, out_dir: &Path, group: &Path, ext: &str) -> PathBuf {
        let mut path = out_dir.join(self.dir()).join(target_and_dataset(group));
        path.as_mut_os_string().push(format!(".{ext}"));
        path
    }
}

/// `<platform>-<viewport>/<dataset>` for a frame directory laid out that way.
///
/// Naming, not a contract: a directory of another shape still renders,
/// under the flatter name its own layout gives it.
fn target_and_dataset(group: &Path) -> PathBuf {
    let parts: Vec<_> = group.iter().collect();
    let [platform, viewport, dataset] = parts.as_slice() else {
        return group.to_owned();
    };
    let mut target = (*platform).to_os_string();
    target.push("-");
    target.push(viewport);
    Path::new(&target).join(dataset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_becomes_one_directory() {
        assert_eq!(
            Media::Comparison.path(
                Path::new("out/clock"),
                Path::new("bmc100/full/qualifying"),
                "mp4",
            ),
            PathBuf::from("out/clock/comparison/bmc100-full/qualifying.mp4"),
        );
    }

    #[test]
    fn a_dataset_named_with_a_dot_keeps_all_of_it() {
        assert_eq!(
            Media::Preview.path(Path::new("out"), Path::new("bmc100/full/v1.2"), "mp4"),
            PathBuf::from("out/preview/bmc100-full/v1.2.mp4"),
        );
    }

    #[test]
    fn a_directory_of_another_shape_still_gets_a_name() {
        assert_eq!(
            Media::Preview.path(Path::new("out"), Path::new("legacy/full"), "mp4"),
            PathBuf::from("out/preview/legacy/full.mp4"),
        );
    }
}
