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

//! Merges fragment files staged under `merge-files/` into a single generated
//! file per target path.
//!
//! Fragments are concatenated as raw bytes in sorted filename order. The merge
//! is line-oriented: when a fragment does not end in a newline, a newline is
//! inserted before the next fragment so successive fragments never run together
//! on the same line.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

fn main() -> anyhow::Result<()> {
    let gen_path_str = std::env::var("PROFILE_NEW_GENERATION")
        .map_err(|_| anyhow::anyhow!("PROFILE_NEW_GENERATION environment variable must be set"))?;
    let gen_path = Path::new(&gen_path_str);
    run(gen_path)
}

fn run(gen_path: &Path) -> anyhow::Result<()> {
    let merge_dir = gen_path.join("merge-files");

    if !merge_dir.exists() {
        return Ok(());
    }

    // Collect all leaf files under merge-files/, grouped by their parent directory
    // (which becomes the target path in the generation root).
    let mut groups: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();

    // The union build links single-provider directories and leaves (including the
    // merge-files root itself) as symlinks into the store, so fragments from
    // multiple packages live behind symlinks. Follow them so the fragments are
    // actually merged; walkdir yields an error when a symlink loop is
    // encountered, which propagates out of the hook.
    for entry in WalkDir::new(&merge_dir).follow_links(true) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(&merge_dir)
                .expect("BUG: entry must be under merge_dir");
            let target = rel
                .parent()
                .expect("BUG: file must have a parent directory");
            groups
                .entry(target.to_path_buf())
                .or_default()
                .push(entry.path().to_path_buf());
        }
    }

    for (target, mut files) in groups {
        files.sort();

        let mut content: Vec<u8> = Vec::new();
        for file in &files {
            if !content.is_empty() && content.last() != Some(&b'\n') {
                content.push(b'\n');
            }
            content.extend_from_slice(&std::fs::read(file)?);
        }

        bmc_nix::generation_path::write_generated_file(gen_path, &target, &content, 0o644)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn run_processes_symlinked_merge_files_root_and_does_not_mutate_store() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let generation = tmp.path().join("generation");
        let store = tmp.path().join("store");
        let store_merge = store.join("merge-files");
        let store_etc = store.join("etc");
        std::fs::create_dir_all(store_merge.join("etc/banner")).expect("BUG: create merge input");
        std::fs::write(store_merge.join("etc/banner/10-a"), "from merge\n")
            .expect("BUG: write merge input");
        std::fs::create_dir_all(&store_etc).expect("BUG: create store etc");
        std::fs::write(store_etc.join("banner"), "store banner\n")
            .expect("BUG: write store banner");
        std::fs::create_dir_all(&generation).expect("BUG: create generation");
        std::os::unix::fs::symlink(&store_merge, generation.join("merge-files"))
            .expect("BUG: symlink merge-files");
        std::os::unix::fs::symlink(&store_etc, generation.join("etc")).expect("BUG: symlink etc");

        super::run(&generation).expect("BUG: merge-files hook should succeed");

        let output = generation.join("etc/banner");
        let meta = output.symlink_metadata().expect("BUG: stat merged output");
        assert!(
            meta.is_file(),
            "merged output should be a generated regular file"
        );
        assert!(
            !meta.file_type().is_symlink(),
            "merged output should not be a symlink into the store"
        );
        assert_eq!(
            std::fs::read_to_string(&output).expect("BUG: read merged output"),
            "from merge\n"
        );
        assert_eq!(
            std::fs::read_to_string(store_etc.join("banner")).expect("BUG: read store banner"),
            "store banner\n",
            "store-backed output must not be modified"
        );
    }

    #[test]
    fn run_merges_fragments_behind_symlinked_leaves() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let generation = tmp.path().join("generation");
        let store = tmp.path().join("store");
        std::fs::create_dir_all(store.join("a")).expect("BUG: create store a");
        std::fs::create_dir_all(store.join("b")).expect("BUG: create store b");
        std::fs::write(store.join("a/10-a"), "alpha\n").expect("BUG: write fragment a");
        std::fs::write(store.join("b/20-b"), "beta\n").expect("BUG: write fragment b");

        let banner = generation.join("merge-files/etc/banner");
        std::fs::create_dir_all(&banner).expect("BUG: create merge dirs");
        std::os::unix::fs::symlink(store.join("a/10-a"), banner.join("10-a"))
            .expect("BUG: symlink fragment a");
        std::os::unix::fs::symlink(store.join("b/20-b"), banner.join("20-b"))
            .expect("BUG: symlink fragment b");

        super::run(&generation).expect("BUG: merge-files hook should succeed");

        let output = generation.join("etc/banner");
        assert!(
            !output
                .symlink_metadata()
                .expect("BUG: stat merged output")
                .file_type()
                .is_symlink(),
            "merged output should be a generated regular file"
        );
        assert_eq!(
            std::fs::read_to_string(&output).expect("BUG: read merged output"),
            "alpha\nbeta\n",
            "fragments behind symlinks must be followed and concatenated in order"
        );
    }

    #[test]
    fn run_merges_fragments_behind_symlinked_directories() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let generation = tmp.path().join("generation");
        let store = tmp.path().join("store");
        std::fs::create_dir_all(store.join("frag/etc/banner")).expect("BUG: create store frag");
        std::fs::write(store.join("frag/etc/banner/10-a"), "gamma\n").expect("BUG: write fragment");

        std::fs::create_dir_all(generation.join("merge-files")).expect("BUG: create merge root");
        std::os::unix::fs::symlink(store.join("frag/etc"), generation.join("merge-files/etc"))
            .expect("BUG: symlink merge subdir");

        super::run(&generation).expect("BUG: merge-files hook should succeed");

        assert_eq!(
            std::fs::read_to_string(generation.join("etc/banner"))
                .expect("BUG: read merged output"),
            "gamma\n",
            "a symlinked merge-files subdirectory must be traversed"
        );
    }

    #[test]
    fn merges_non_utf8_fragments() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let generation = tmp.path().join("generation");
        let banner = generation.join("merge-files/etc/banner");
        std::fs::create_dir_all(&banner).expect("BUG: create merge dirs");
        std::fs::write(banner.join("10-a"), [0xFF, 0xFE]).expect("BUG: write fragment a");
        std::fs::write(banner.join("20-b"), [0x00, 0xFF]).expect("BUG: write fragment b");

        super::run(&generation).expect("BUG: merge-files hook should succeed");

        assert_eq!(
            std::fs::read(generation.join("etc/banner")).expect("BUG: read merged output"),
            [0xFF, 0xFE, b'\n', 0x00, 0xFF],
            "non-UTF-8 fragments must merge byte-for-byte"
        );
    }

    #[test]
    fn inserts_newline_between_unterminated_fragments() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let generation = tmp.path().join("generation");
        let banner = generation.join("merge-files/etc/banner");
        std::fs::create_dir_all(&banner).expect("BUG: create merge dirs");
        std::fs::write(banner.join("10-a"), "a").expect("BUG: write fragment a");
        std::fs::write(banner.join("20-b"), "b").expect("BUG: write fragment b");

        super::run(&generation).expect("BUG: merge-files hook should succeed");

        assert_eq!(
            std::fs::read_to_string(generation.join("etc/banner"))
                .expect("BUG: read merged output"),
            "a\nb",
            "unterminated fragments must be newline-separated"
        );
    }

    #[test]
    fn run_errors_on_symlinked_directory_cycle() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let generation = tmp.path().join("generation");
        std::fs::create_dir_all(generation.join("merge-files")).expect("BUG: create merge root");
        std::os::unix::fs::symlink(".", generation.join("merge-files/self"))
            .expect("BUG: create cyclic symlink");

        let result = super::run(&generation);

        assert!(
            result.is_err(),
            "a symlink loop under merge-files must error, not loop forever"
        );
    }
}
