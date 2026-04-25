// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use tracing::{debug, info};

const COPY_DIR: &str = "special/copy";

/// A file to be copied, with pre-collected metadata.
struct FileEntry {
    source: PathBuf,
    relative: PathBuf,
    size: u64,
    permissions: std::fs::Permissions,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let gen_path_str = std::env::var("PROFILE_NEW_GENERATION")
        .map_err(|_| anyhow::anyhow!("PROFILE_NEW_GENERATION environment variable must be set"))?;
    let gen_path = Path::new(&gen_path_str);
    info!(path = %gen_path.display(), "new generation");

    let old_gen_str = std::env::var("PROFILE_OLD_GENERATION").unwrap_or_default();
    let old_gen_path = if old_gen_str.is_empty() {
        info!("no old generation");
        None
    } else {
        info!(path = %old_gen_str, "old generation");
        Some(Path::new(&old_gen_str).to_path_buf())
    };

    let copy_dir = gen_path.join(COPY_DIR);
    if !copy_dir.exists() {
        info!(path = %copy_dir.display(), "copy dir does not exist, nothing to do");
        return Ok(());
    }
    debug!(path = %copy_dir.display(), "copy dir found");

    info!("cleaning up stale files");
    cleanup_stale_files(
        old_gen_path.as_deref().map(|p| p.join(COPY_DIR)).as_deref(),
        &copy_dir,
        Path::new("/"),
    );

    info!("collecting files to copy");
    let entries = collect_file_entries(&copy_dir)?;

    info!("checking filesystem space");
    let free_bytes = statvfs_free_bytes(Path::new("/"))?;
    check_space(&entries, Path::new("/"), free_bytes)?;

    info!("copying files");
    copy_files(&entries, Path::new("/"))?;

    info!("all phases complete");
    Ok(())
}

fn cleanup_stale_files(old_copy_dir: Option<&Path>, new_copy_dir: &Path, target_root: &Path) {
    let old_copy_dir = match old_copy_dir {
        Some(dir) if dir.exists() => dir,
        _ => return,
    };

    for entry in walkdir::WalkDir::new(old_copy_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(old_copy_dir)
            .expect("BUG: entry must be under old_copy_dir");
        if !new_copy_dir.join(rel).exists() {
            let target = target_root.join(rel);
            debug!(path = %target.display(), "removing stale file");
            if let Err(err) = std::fs::remove_file(&target)
                && err.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(
                    path = %target.display(),
                    %err,
                    "failed to remove stale file"
                );
            }
        }
    }
}

fn collect_file_entries(copy_dir: &Path) -> anyhow::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    for entry in walkdir::WalkDir::new(copy_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(copy_dir)
            .expect("BUG: entry must be under copy_dir");
        let metadata = entry.metadata()?;
        entries.push(FileEntry {
            source: entry.path().to_path_buf(),
            relative: rel.to_path_buf(),
            size: metadata.len(),
            permissions: metadata.permissions(),
        });
    }
    Ok(entries)
}

const MIN_FREE_BYTES: u64 = 1024 * 1024;

fn check_space(entries: &[FileEntry], target_root: &Path, free_bytes: u64) -> anyhow::Result<()> {
    let total_new: u64 = entries.iter().map(|e| e.size).sum();
    // Use symlink_metadata: when the existing target is a symlink, atomic
    // rename replaces the symlink itself (not the file it points to), so
    // only the symlink's own bytes are reclaimed.
    let total_existing: u64 = entries
        .iter()
        .filter_map(|e| {
            let target = target_root.join(&e.relative);
            std::fs::symlink_metadata(&target).ok().map(|m| m.len())
        })
        .sum();

    // Peak temporary usage: the net delta (new minus reclaimed) plus the
    // largest new file, because each replacement briefly keeps the old
    // target alongside its `.tmp` sibling until `rename` swaps them — so
    // the largest file must fit on disk on top of the net growth.
    let largest_new: u64 = entries.iter().map(|e| e.size).max().unwrap_or(0);
    let required = total_new
        .saturating_sub(total_existing)
        .saturating_add(largest_new);

    anyhow::ensure!(
        free_bytes.saturating_sub(required) >= MIN_FREE_BYTES,
        "insufficient disk space: {free_bytes} bytes free, need {required} bytes \
         plus {MIN_FREE_BYTES} byte safety margin"
    );

    Ok(())
}

fn statvfs_free_bytes(path: &Path) -> anyhow::Result<u64> {
    use std::ffi::CString;

    let c_path = CString::new(
        path.to_str()
            .ok_or_else(|| anyhow::anyhow!("path is not valid UTF-8: {}", path.display()))?,
    )?;

    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::statvfs(c_path.as_ptr(), &raw mut stat) };

    anyhow::ensure!(
        ret == 0,
        "statvfs failed for {}: {}",
        path.display(),
        std::io::Error::last_os_error()
    );

    // Widen to u64 before multiplication to prevent overflow on 32-bit ARM
    // where c_ulong is u32. On x86_64 these are already u64.
    #[cfg_attr(
        target_pointer_width = "64",
        expect(
            clippy::useless_conversion,
            reason = "needed on 32-bit ARM where c_ulong is u32"
        )
    )]
    Ok(u64::from(stat.f_frsize) * u64::from(stat.f_bavail))
}

fn copy_files(entries: &[FileEntry], target_root: &Path) -> anyhow::Result<()> {
    let mut created_dirs: HashSet<PathBuf> = HashSet::new();
    for entry in entries {
        let target = target_root.join(&entry.relative);
        debug!(src = %entry.source.display(), dst = %target.display(), "copying file");
        if let Some(parent) = target.parent()
            && created_dirs.insert(parent.to_path_buf())
        {
            std::fs::create_dir_all(parent)?;
        }
        copy_file_atomic(&entry.source, &target, &entry.permissions)?;
    }
    info!(count = entries.len(), "files copied");
    Ok(())
}

fn copy_file_atomic(
    source: &Path,
    target: &Path,
    permissions: &std::fs::Permissions,
) -> anyhow::Result<()> {
    let parent = target.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "target path must have a parent directory: {}",
            target.display()
        )
    })?;

    match files_contents_match(source, target) {
        Ok(true) => {
            debug!(
                src = %source.display(),
                dst = %target.display(),
                "contents match, skipping copy"
            );
            return Ok(());
        }
        Ok(false) => {}
        Err(err) => {
            debug!(
                src = %source.display(),
                dst = %target.display(),
                %err,
                "contents comparison failed, proceeding with copy"
            );
        }
    }

    let temp_path = temp_path_for_target(parent, target)?;

    match std::fs::remove_file(&temp_path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(Into::into(err));
        }
    }

    let result = (|| -> anyhow::Result<()> {
        let mut source_file = File::open(source)?;
        let mut temp_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)?;

        io::copy(&mut source_file, &mut temp_file)?;
        temp_file.sync_all()?;
        drop(temp_file);

        std::fs::set_permissions(&temp_path, permissions.clone())?;
        std::fs::rename(&temp_path, target)?;

        File::open(parent)?.sync_all()?;
        Ok(())
    })();

    if result.is_err() {
        match std::fs::remove_file(&temp_path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                tracing::warn!(path = %temp_path.display(), %err, "failed to remove temp file");
            }
        }
    }

    result
}

/// Return `true` if `source` and `target` exist and have identical contents.
///
/// Streams both files through `BufReader`s and compares chunks in place, so
/// peak memory stays bounded to the buffer size regardless of file size and
/// the comparison short-circuits on the first differing byte.
///
/// Returns an IO error when either file cannot be opened (e.g. target missing)
/// so the caller can fall back to performing the copy.
fn files_contents_match(source: &Path, target: &Path) -> io::Result<bool> {
    let source_meta = std::fs::metadata(source)?;
    let target_meta = std::fs::metadata(target)?;
    if source_meta.len() != target_meta.len() {
        return Ok(false);
    }
    let mut a = BufReader::new(File::open(source)?);
    let mut b = BufReader::new(File::open(target)?);
    loop {
        let buf_a = a.fill_buf()?;
        let buf_b = b.fill_buf()?;
        if buf_a.is_empty() && buf_b.is_empty() {
            return Ok(true);
        }
        let n = buf_a.len().min(buf_b.len());
        if n == 0 {
            // One side still has data, the other hit EOF — sizes matched so
            // this means a short read; treat as not-matching and let the
            // copy path rewrite the file from scratch.
            return Ok(false);
        }
        if buf_a[..n] != buf_b[..n] {
            return Ok(false);
        }
        a.consume(n);
        b.consume(n);
    }
}

fn temp_path_for_target(parent: &Path, target: &Path) -> anyhow::Result<PathBuf> {
    let file_name = target.file_name().ok_or_else(|| {
        anyhow::anyhow!(
            "target path must end with a file name: {}",
            target.display()
        )
    })?;

    Ok(parent.join(format!(".{}.tmp", file_name.to_string_lossy())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn create_file(base: &Path, rel: &str, content: &str) {
        let path = base.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("BUG: failed to create parent dirs in test");
        }
        std::fs::write(&path, content).expect("BUG: failed to write file in test");
    }

    #[test]
    fn removes_stale_files_from_old_generation() {
        let tmp = TempDir::new().expect("BUG: create temp dir");
        let old_copy = tmp.path().join("old/special/copy");
        create_file(&old_copy, "etc/init.d/old-service", "old");
        create_file(&old_copy, "etc/init.d/kept-service", "kept");
        let new_copy = tmp.path().join("new/special/copy");
        create_file(&new_copy, "etc/init.d/kept-service", "kept-new");
        let target_root = tmp.path().join("target");
        create_file(&target_root, "etc/init.d/old-service", "old");
        create_file(&target_root, "etc/init.d/kept-service", "kept");

        cleanup_stale_files(Some(&old_copy), &new_copy, &target_root);

        assert!(!target_root.join("etc/init.d/old-service").exists());
        assert!(target_root.join("etc/init.d/kept-service").exists());
    }

    #[test]
    fn cleanup_handles_no_old_generation() {
        let tmp = TempDir::new().expect("BUG: create temp dir");
        let new_copy = tmp.path().join("new/special/copy");
        create_file(&new_copy, "etc/init.d/service", "content");
        let target_root = tmp.path().join("target");

        cleanup_stale_files(None, &new_copy, &target_root);
    }

    #[test]
    fn space_check_passes_with_plenty_of_space() {
        let tmp = TempDir::new().expect("BUG: create temp dir");
        let copy_dir = tmp.path().join(COPY_DIR);
        create_file(&copy_dir, "etc/small-file", "hello");
        let target_root = tmp.path().join("target");

        let entries = collect_file_entries(&copy_dir).expect("BUG: collect entries");
        check_space(&entries, &target_root, 1024 * 1024 * 1024)
            .expect("BUG: space check should pass");
    }

    #[test]
    fn copies_files_to_target_paths() {
        let tmp = TempDir::new().expect("BUG: create temp dir");
        let copy_dir = tmp.path().join(COPY_DIR);
        create_file(&copy_dir, "etc/init.d/my-service", "#!/bin/sh\necho hi");
        create_file(&copy_dir, "root/.profile", "export PATH=$PATH:/nix/bin");
        let target_root = tmp.path().join("target");

        let entries = collect_file_entries(&copy_dir).expect("BUG: collect entries");
        copy_files(&entries, &target_root).expect("BUG: copy should succeed");

        assert_eq!(
            std::fs::read_to_string(target_root.join("etc/init.d/my-service"))
                .expect("BUG: read target"),
            "#!/bin/sh\necho hi"
        );
        assert_eq!(
            std::fs::read_to_string(target_root.join("root/.profile")).expect("BUG: read target"),
            "export PATH=$PATH:/nix/bin"
        );
    }

    #[test]
    fn copies_overwrite_existing_files() {
        let tmp = TempDir::new().expect("BUG: create temp dir");
        let copy_dir = tmp.path().join(COPY_DIR);
        create_file(&copy_dir, "etc/config", "new-content");
        let target_root = tmp.path().join("target");
        create_file(&target_root, "etc/config", "old-content");

        let entries = collect_file_entries(&copy_dir).expect("BUG: collect entries");
        copy_files(&entries, &target_root).expect("BUG: copy should succeed");

        assert_eq!(
            std::fs::read_to_string(target_root.join("etc/config")).expect("BUG: read target"),
            "new-content"
        );
    }

    #[test]
    fn atomic_copy_replaces_target_via_rename() {
        let tmp = TempDir::new().expect("BUG: create temp dir");
        let source = tmp.path().join("source");
        let target = tmp.path().join("target");
        std::fs::write(&source, "new-content").expect("BUG: write source");
        std::fs::write(&target, "old-content").expect("BUG: write target");

        let permissions = std::fs::metadata(&source)
            .expect("BUG: source metadata")
            .permissions();
        let old_inode = std::fs::metadata(&target)
            .expect("BUG: target metadata")
            .ino();

        copy_file_atomic(&source, &target, &permissions).expect("BUG: atomic copy should succeed");

        let new_inode = std::fs::metadata(&target)
            .expect("BUG: target metadata after copy")
            .ino();
        assert_ne!(old_inode, new_inode);
        assert_eq!(
            std::fs::read_to_string(&target).expect("BUG: read target after copy"),
            "new-content"
        );
    }

    #[test]
    fn atomic_copy_skips_when_contents_match() {
        let tmp = TempDir::new().expect("BUG: create temp dir");
        let source = tmp.path().join("source");
        let target = tmp.path().join("target");
        std::fs::write(&source, "same-content").expect("BUG: write source");
        std::fs::write(&target, "same-content").expect("BUG: write target");

        let permissions = std::fs::metadata(&source)
            .expect("BUG: source metadata")
            .permissions();
        let old_inode = std::fs::metadata(&target)
            .expect("BUG: target metadata")
            .ino();

        copy_file_atomic(&source, &target, &permissions).expect("BUG: atomic copy should succeed");

        let new_inode = std::fs::metadata(&target)
            .expect("BUG: target metadata after copy")
            .ino();
        assert_eq!(
            old_inode, new_inode,
            "target inode must be preserved when contents match"
        );
        assert_eq!(
            std::fs::read_to_string(&target).expect("BUG: read target after copy"),
            "same-content"
        );
    }

    #[test]
    fn atomic_copy_cleans_up_temp_file_on_rename_failure() {
        // rename(2) fails with EISDIR when renaming a regular file over an
        // existing directory. This exercises the error-path cleanup that
        // removes the temp file when the inner write sequence returns Err.
        let tmp = TempDir::new().expect("BUG: create temp dir");
        let source = tmp.path().join("source");
        std::fs::write(&source, "content").expect("BUG: write source");

        let target = tmp.path().join("target");
        std::fs::create_dir(&target).expect("BUG: create target dir");
        std::fs::write(target.join("child"), "keep").expect("BUG: write child");

        let permissions = std::fs::metadata(&source)
            .expect("BUG: source metadata")
            .permissions();

        let result = copy_file_atomic(&source, &target, &permissions);
        assert!(
            result.is_err(),
            "rename of file over non-empty directory must fail"
        );

        let temp_path = temp_path_for_target(tmp.path(), &target).expect("BUG: temp path");
        assert!(
            !temp_path.exists(),
            "temp file must be cleaned up after copy failure"
        );
        // Target directory is left intact; its child is untouched.
        assert!(target.is_dir(), "target directory preserved on failure");
        assert_eq!(
            std::fs::read_to_string(target.join("child")).expect("BUG: read child"),
            "keep"
        );
    }

    #[test]
    fn boot_time_reactivation_is_idempotent() {
        // Simulates the boot path in files/nix-activator: on every boot the
        // activator runs with PROFILE_OLD_GENERATION="" (None) and an
        // unchanged generation.  The target rootfs already has the right
        // files from a previous activation, so re-running the full sequence
        // must be a no-op at the inode level (no rewrites, no stale state).
        let tmp = TempDir::new().expect("BUG: create temp dir");
        let target_root = tmp.path().join("target");

        let new_copy = tmp.path().join("new/special/copy");
        create_file(&new_copy, "etc/init.d/nix-mounter", "mounter-content");
        create_file(&new_copy, "root/.profile", "profile-content");

        // Pre-populate target as though a previous activation already ran.
        create_file(&target_root, "etc/init.d/nix-mounter", "mounter-content");
        create_file(&target_root, "root/.profile", "profile-content");

        let paths = ["etc/init.d/nix-mounter", "root/.profile"];
        let pre_inodes: Vec<u64> = paths
            .iter()
            .map(|p| {
                std::fs::metadata(target_root.join(p))
                    .expect("BUG: target metadata")
                    .ino()
            })
            .collect();

        // Run the full main-flow with no old generation (boot scenario).
        cleanup_stale_files(None, &new_copy, &target_root);
        let entries = collect_file_entries(&new_copy).expect("BUG: collect entries");
        check_space(&entries, &target_root, 1024 * 1024 * 1024).expect("BUG: space check");
        copy_files(&entries, &target_root).expect("BUG: copy");

        let post_inodes: Vec<u64> = paths
            .iter()
            .map(|p| {
                std::fs::metadata(target_root.join(p))
                    .expect("BUG: target metadata")
                    .ino()
            })
            .collect();

        assert_eq!(
            pre_inodes, post_inodes,
            "boot reactivation must not rewrite files whose contents match"
        );
    }

    #[test]
    fn atomic_copy_removes_stale_temp_file_before_reuse() {
        let tmp = TempDir::new().expect("BUG: create temp dir");
        let source = tmp.path().join("source");
        let target = tmp.path().join("target");
        std::fs::write(&source, "new-content").expect("BUG: write source");
        std::fs::write(&target, "old-content").expect("BUG: write target");

        let temp_path =
            temp_path_for_target(tmp.path(), &target).expect("BUG: temp path should resolve");
        std::fs::write(&temp_path, "stale-temp").expect("BUG: write stale temp file");

        let permissions = std::fs::metadata(&source)
            .expect("BUG: source metadata")
            .permissions();

        copy_file_atomic(&source, &target, &permissions).expect("BUG: atomic copy should succeed");

        assert!(!temp_path.exists());
        assert_eq!(
            std::fs::read_to_string(&target).expect("BUG: read target after copy"),
            "new-content"
        );
    }

    #[test]
    fn copies_executable_bits_from_source_files() {
        let tmp = TempDir::new().expect("BUG: create temp dir");
        let copy_dir = tmp.path().join(COPY_DIR);
        create_file(&copy_dir, "etc/init.d/nix-mounter", "#!/bin/sh\nexit 0");
        let source = copy_dir.join("etc/init.d/nix-mounter");
        let mut permissions = std::fs::metadata(&source)
            .expect("BUG: source metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&source, permissions).expect("BUG: set source permissions");

        let target_root = tmp.path().join("target");
        let entries = collect_file_entries(&copy_dir).expect("BUG: collect entries");
        copy_files(&entries, &target_root).expect("BUG: copy should succeed");

        let copied_mode = std::fs::metadata(target_root.join("etc/init.d/nix-mounter"))
            .expect("BUG: copied metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(copied_mode, 0o755);
    }

    #[test]
    fn full_copy_files_flow() {
        let tmp = TempDir::new().expect("BUG: create temp dir");
        let target_root = tmp.path().join("target");

        let old_copy = tmp.path().join("old/special/copy");
        create_file(&old_copy, "etc/init.d/nix-mounter", "old-mounter");
        create_file(&old_copy, "etc/init.d/removed-service", "to-remove");
        create_file(&old_copy, "root/.profile", "old-profile");

        create_file(&target_root, "etc/init.d/nix-mounter", "old-mounter");
        create_file(&target_root, "etc/init.d/removed-service", "to-remove");
        create_file(&target_root, "root/.profile", "old-profile");

        let new_copy = tmp.path().join("new/special/copy");
        create_file(&new_copy, "etc/init.d/nix-mounter", "new-mounter");
        create_file(&new_copy, "root/.profile", "old-profile");

        cleanup_stale_files(Some(&old_copy), &new_copy, &target_root);
        let entries = collect_file_entries(&new_copy).expect("BUG: collect entries");
        check_space(&entries, &target_root, 1024 * 1024 * 1024)
            .expect("BUG: space check should pass");
        copy_files(&entries, &target_root).expect("BUG: copy should succeed");

        assert!(!target_root.join("etc/init.d/removed-service").exists());
        assert_eq!(
            std::fs::read_to_string(target_root.join("etc/init.d/nix-mounter")).expect("BUG: read"),
            "new-mounter"
        );
        assert_eq!(
            std::fs::read_to_string(target_root.join("root/.profile")).expect("BUG: read"),
            "old-profile"
        );
    }

    #[test]
    fn space_check_uses_symlink_size_not_target() {
        use std::os::unix::fs::symlink;

        let tmp = TempDir::new().expect("BUG: create temp dir");
        let copy_dir = tmp.path().join(COPY_DIR);
        let source_path = copy_dir.join("etc/config");
        std::fs::create_dir_all(source_path.parent().expect("BUG: parent"))
            .expect("BUG: mkdir source parent");
        // 2 MiB sparse source — fast, no real allocation.
        let source = File::create(&source_path).expect("BUG: create source");
        source
            .set_len(2 * 1024 * 1024)
            .expect("BUG: set source len");

        let target_root = tmp.path().join("target");
        let big_file = tmp.path().join("big");
        // 10 MiB sparse file that the target symlink points at.
        let big = File::create(&big_file).expect("BUG: create big");
        big.set_len(10 * 1024 * 1024).expect("BUG: set big len");

        let target_path = target_root.join("etc/config");
        std::fs::create_dir_all(target_path.parent().expect("BUG: parent"))
            .expect("BUG: mkdir target parent");
        symlink(&big_file, &target_path).expect("BUG: create symlink");

        let entries = collect_file_entries(&copy_dir).expect("BUG: collect entries");

        // 3 MiB of free space. Replacing the 2 MiB source needs ~4 MiB of
        // headroom (net delta + largest-new) when the existing symlink is
        // correctly counted as its own (tiny) size. If the code follows
        // the symlink and counts the 10 MiB file instead, the check is
        // fooled into believing the replacement almost pays for itself.
        let free_bytes: u64 = 3 * 1024 * 1024;

        let result = check_space(&entries, &target_root, free_bytes);
        assert!(
            result.is_err(),
            "space check must fail because replacing a symlink only reclaims \
             the symlink's size, not the target file's size; got {result:?}"
        );
    }

    #[test]
    fn copy_replaces_symlink_not_its_target() {
        use std::os::unix::fs::symlink;

        let tmp = TempDir::new().expect("BUG: create temp dir");
        let copy_dir = tmp.path().join(COPY_DIR);
        create_file(&copy_dir, "etc/config", "new-content");

        let target_root = tmp.path().join("target");
        let pointed_at = tmp.path().join("pointed-at");
        std::fs::write(&pointed_at, "original-pointed-at-content")
            .expect("BUG: write pointed-at file");
        let target_path = target_root.join("etc/config");
        std::fs::create_dir_all(target_path.parent().expect("BUG: parent"))
            .expect("BUG: mkdir target parent");
        symlink(&pointed_at, &target_path).expect("BUG: create symlink");

        let pointed_at_inode = std::fs::metadata(&pointed_at)
            .expect("BUG: pointed-at metadata")
            .ino();

        let entries = collect_file_entries(&copy_dir).expect("BUG: collect entries");
        copy_files(&entries, &target_root).expect("BUG: copy should succeed");

        // Target path is now a regular file holding the new content, not
        // a symlink — rename(2) replaces the path entry (the symlink),
        // never the file the symlink used to point at.
        let target_meta =
            std::fs::symlink_metadata(&target_path).expect("BUG: target symlink_metadata");
        assert!(
            target_meta.file_type().is_file(),
            "target must be a regular file, not a symlink; got {:?}",
            target_meta.file_type()
        );
        assert_eq!(
            std::fs::read_to_string(&target_path).expect("BUG: read target"),
            "new-content"
        );

        // The file the symlink used to point at must be untouched — same
        // inode, same content.
        let pointed_at_meta_after =
            std::fs::metadata(&pointed_at).expect("BUG: pointed-at metadata after");
        assert_eq!(
            pointed_at_meta_after.ino(),
            pointed_at_inode,
            "the file the symlink pointed at must not have been replaced"
        );
        assert_eq!(
            std::fs::read_to_string(&pointed_at).expect("BUG: read pointed-at"),
            "original-pointed-at-content",
            "the file the symlink pointed at must be unchanged"
        );
    }
}
