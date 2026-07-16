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

use std::ffi::CString;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Result of a successful `bind_mount_nix` call.
#[derive(Debug, PartialEq, Eq)]
pub enum MountOutcome {
    Mounted,
    AlreadyMounted,
    SourceMissing,
}

/// Failure modes that force a non-zero CLI exit.
#[derive(Debug, Error)]
pub enum MountError {
    #[error("mount target '{path}' exists but is not a directory")]
    TargetNotDirectory { path: PathBuf },
    #[error("failed to create mount target '{path}': {source}")]
    MkdirTarget {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read /proc/self/mountinfo: {0}")]
    ReadMountInfo(#[source] io::Error),
    #[error("bind mount syscall failed: {source}")]
    MountSyscall {
        #[source]
        source: io::Error,
    },
}

const MOUNTINFO_PATH: &str = "/proc/self/mountinfo";

/// Bind-mount `source` onto `target`, or report an idempotent outcome
/// (`AlreadyMounted`) if `target` is already a mount point.
///
/// A missing `source` directory is reported as `SourceMissing`. The CLI
/// layer maps this to a non-zero exit with a distinct log line, matching
/// the shell activator's `mount_nix` early-return behavior.
pub fn bind_mount_nix(source: &Path, target: &Path) -> Result<MountOutcome, MountError> {
    if !source.is_dir() {
        return Ok(MountOutcome::SourceMissing);
    }

    if target.exists() && !target.is_dir() {
        return Err(MountError::TargetNotDirectory {
            path: target.to_path_buf(),
        });
    }

    std::fs::create_dir_all(target).map_err(|source| MountError::MkdirTarget {
        path: target.to_path_buf(),
        source,
    })?;

    let mountinfo = std::fs::read_to_string(MOUNTINFO_PATH).map_err(MountError::ReadMountInfo)?;
    if is_target_mounted(&mountinfo, target) {
        return Ok(MountOutcome::AlreadyMounted);
    }

    do_bind_mount(source, target)?;
    Ok(MountOutcome::Mounted)
}

fn do_bind_mount(source: &Path, target: &Path) -> Result<(), MountError> {
    let source_c = CString::new(source.as_os_str().as_encoded_bytes())
        .expect("BUG: mount source contains NUL byte");
    let target_c = CString::new(target.as_os_str().as_encoded_bytes())
        .expect("BUG: mount target contains NUL byte");

    // SAFETY: source_c/target_c are C strings we own for the duration of
    // the call; fstype/data are NULL, which mount(2) accepts for MS_BIND.
    let rc = unsafe {
        libc::mount(
            source_c.as_ptr(),
            target_c.as_ptr(),
            std::ptr::null(),
            libc::MS_BIND,
            std::ptr::null(),
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(MountError::MountSyscall {
            source: io::Error::last_os_error(),
        })
    }
}

/// Return whether `target` appears as the mount point of any entry in
/// the given `/proc/self/mountinfo` contents.
///
/// Mount points in `mountinfo` are octal-escaped for the special
/// characters space, tab, newline and backslash (per `proc(5)`); this
/// helper decodes them before comparing to `target`.
#[must_use]
pub fn is_target_mounted(mountinfo: &str, target: &Path) -> bool {
    for line in mountinfo.lines() {
        let Some(mount_point) = line.split_whitespace().nth(4) else {
            continue;
        };
        let decoded = decode_mount_field(mount_point);
        if Path::new(&decoded) == target {
            return true;
        }
    }
    false
}

/// Decode the octal escapes (`\NNN`) that `proc(5)` uses for special
/// characters in mount-table paths and source devices.
///
/// A multi-byte UTF-8 code point is escaped one byte at a time, so the
/// decoded bytes are accumulated and interpreted as UTF-8 only at the
/// end; decoding per character would corrupt any non-ASCII path.
pub(crate) fn decode_mount_field(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let (a, b, c) = (bytes[i + 1], bytes[i + 2], bytes[i + 3]);
            if a.is_ascii_digit() && b.is_ascii_digit() && c.is_ascii_digit() {
                let val = ((a - b'0') << 6) + ((b - b'0') << 3) + (c - b'0');
                out.push(val);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
25 30 0:22 / /proc rw,nosuid,nodev,noexec,relatime shared:12 - proc proc rw
26 30 0:23 / /sys rw,nosuid,nodev,noexec,relatime shared:13 - sysfs sysfs rw
40 30 253:0 /mnt/data/nix /nix rw,relatime shared:1 - ext4 /dev/mmcblk0p4 rw
41 30 253:0 / /nix/store ro,relatime shared:2 - ext4 /dev/mmcblk0p4 rw
";

    #[test]
    fn detects_target_present() {
        assert!(is_target_mounted(SAMPLE, Path::new("/nix")));
    }

    #[test]
    fn ignores_target_appearing_only_as_child_of_other_mount() {
        assert!(!is_target_mounted(SAMPLE, Path::new("/var")));
    }

    #[test]
    fn detects_target_that_is_child_mount() {
        assert!(is_target_mounted(SAMPLE, Path::new("/nix/store")));
    }

    #[test]
    fn absent_target_returns_false() {
        assert!(!is_target_mounted(SAMPLE, Path::new("/does/not/exist")));
    }

    #[test]
    fn decodes_octal_escaped_spaces() {
        let mountinfo = "40 30 253:0 / /mnt/with\\040space rw - ext4 /dev/x rw\n";
        assert!(is_target_mounted(mountinfo, Path::new("/mnt/with space")));
    }

    #[test]
    fn ignores_blank_and_short_lines() {
        assert!(!is_target_mounted("\n\nfoo bar\n", Path::new("/nix")));
    }

    #[test]
    fn octal_decoding_is_byte_accurate() {
        assert_eq!(decode_mount_field("caf\\303\\251"), "café");
    }

    #[test]
    fn bind_mount_source_missing_returns_source_missing() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let source = dir.path().join("nonexistent");
        let target = dir.path().join("target");
        match bind_mount_nix(&source, &target) {
            Ok(MountOutcome::SourceMissing) => {}
            other => panic!("expected SourceMissing, got {other:?}"),
        }
    }

    #[test]
    fn bind_mount_target_is_regular_file_returns_target_not_directory() {
        let dir = tempfile::tempdir().expect("BUG: tempdir");
        let source = dir.path().join("source");
        std::fs::create_dir(&source).expect("BUG: mk source");
        let target = dir.path().join("target");
        std::fs::write(&target, b"not a directory").expect("BUG: write target");
        match bind_mount_nix(&source, &target) {
            Err(MountError::TargetNotDirectory { .. }) => {}
            other => panic!("expected TargetNotDirectory, got {other:?}"),
        }
    }
}
