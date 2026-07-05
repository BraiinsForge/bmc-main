// Copyright (C) 2026  Braiins Systems s.r.o.

//! Activation write boundary (prefix 050): durably flip the `current`
//! symlink to the new generation.
//!
//! Replaces the former shell entry, which could not satisfy the
//! fail-loud durability contract: `sync(1)` reports no per-filesystem
//! writeback errors and POSIX sh cannot fsync a directory. Ordering:
//! syncfs the profile filesystem (everything earlier activation steps
//! and the generation build wrote), flip `current` via tmp + rename,
//! fsync the profile dir so the flip survives a crash before success
//! is reported. Every failure exits non-zero and fails the activation.

use std::path::Path;

use tracing::info;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let gen_path_str = std::env::var("PROFILE_NEW_GENERATION")
        .map_err(|_| anyhow::anyhow!("PROFILE_NEW_GENERATION environment variable must be set"))?;
    let gen_path = Path::new(&gen_path_str);
    let profile_dir = gen_path.parent().ok_or_else(|| {
        anyhow::anyhow!("generation path must have a parent directory: {gen_path_str}")
    })?;
    let gen_dir_name = gen_path.file_name().ok_or_else(|| {
        anyhow::anyhow!("generation path must end with a directory name: {gen_path_str}")
    })?;

    info!(generation = %gen_path.display(), "flipping current");

    bmc_nix::fs_sync::sync_filesystem_of(profile_dir)?;
    flip_current(profile_dir, Path::new(gen_dir_name))?;
    bmc_nix::fs_sync::fsync_dir(profile_dir)?;

    Ok(())
}

/// Atomically point `<profile_dir>/current` at `gen_dir_name` via a
/// `current.tmp` symlink and rename, removing a stale tmp link first.
fn flip_current(profile_dir: &Path, gen_dir_name: &Path) -> std::io::Result<()> {
    let tmp_link = profile_dir.join("current.tmp");
    match std::fs::remove_file(&tmp_link) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    std::os::unix::fs::symlink(gen_dir_name, &tmp_link)?;
    std::fs::rename(&tmp_link, profile_dir.join("current"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flip_creates_relative_current_symlink() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        flip_current(tmp.path(), Path::new("3-link")).expect("BUG: flip should succeed");

        let target = std::fs::read_link(tmp.path().join("current")).expect("BUG: read link");
        assert_eq!(target, Path::new("3-link"));
        assert!(!tmp.path().join("current.tmp").exists());
    }

    #[test]
    fn flip_replaces_existing_current() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        std::os::unix::fs::symlink("2-link", tmp.path().join("current"))
            .expect("BUG: setup current");

        flip_current(tmp.path(), Path::new("3-link")).expect("BUG: flip should succeed");

        let target = std::fs::read_link(tmp.path().join("current")).expect("BUG: read link");
        assert_eq!(target, Path::new("3-link"));
    }

    #[test]
    fn flip_removes_stale_tmp_link() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        std::os::unix::fs::symlink("1-link", tmp.path().join("current.tmp"))
            .expect("BUG: setup stale tmp");

        flip_current(tmp.path(), Path::new("3-link")).expect("BUG: flip should succeed");

        let target = std::fs::read_link(tmp.path().join("current")).expect("BUG: read link");
        assert_eq!(target, Path::new("3-link"));
        assert!(!tmp.path().join("current.tmp").exists());
    }
}
