// Copyright (C) 2026  Braiins Systems s.r.o.

use std::path::Path;

use crate::store::{CommandRunner, path_string, stderr_snippet};

#[derive(Debug, thiserror::Error)]
pub enum PreparePartitionError {
    #[error("data partition does not exist or is not a block device: {partition}")]
    NoDataPartition { partition: String },
    #[error("failed to run {program}: {source}")]
    CommandFailed {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{program} exited with {status}: {stderr}")]
    CommandExited {
        program: String,
        status: std::process::ExitStatus,
        stderr: String,
    },
    #[error("failed to create mount point {path}: {source}")]
    CreateMountPoint {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read /proc/mounts: {source}")]
    ReadMounts {
        #[source]
        source: std::io::Error,
    },
    #[error("mount point {mount_point} already holds {mounted}, but {requested} was requested")]
    ForeignMount {
        mount_point: String,
        requested: String,
        mounted: String,
    },
    #[error("data partition {partition} is mounted; refusing to fsck or format it")]
    PartitionMounted { partition: String },
    #[error("failed to read /proc/self/mountinfo: {source}")]
    ReadMountInfo {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to stat data partition {partition}: {source}")]
    StatDataPartition {
        partition: String,
        #[source]
        source: std::io::Error,
    },
}

async fn run_command(
    runner: &impl CommandRunner,
    program: &str,
    args: &[&str],
) -> Result<std::process::Output, PreparePartitionError> {
    runner
        .run(program, args)
        .await
        .map_err(|source| PreparePartitionError::CommandFailed {
            program: program.to_owned(),
            source,
        })
}

fn require_success(
    program: &str,
    output: &std::process::Output,
) -> Result<(), PreparePartitionError> {
    if output.status.success() {
        return Ok(());
    }

    Err(PreparePartitionError::CommandExited {
        program: program.to_owned(),
        status: output.status,
        stderr: stderr_snippet(&output.stderr),
    })
}

/// e2fsck's exit status interpreted per its bitmask contract: bit 1
/// errors corrected, bit 2 reboot recommended, bit 4 errors left
/// uncorrected, bits 8/16/32/128 operational, usage, cancel, and
/// library failures.
enum E2fsckOutcome {
    /// Bits within {1, 2}: clean or fully repaired. The reboot advice
    /// behind bit 2 targets mounted filesystems; this partition is only
    /// fscked while unmounted, so the kernel holds no stale metadata and
    /// the filesystem is immediately safe to mount.
    Clean,
    /// Bit 4 with nothing above it: corruption this e2fsck mode could
    /// not fix.
    Corrupt,
    /// Any higher or unknown bit, or death by signal: a tool or device
    /// failure, not a filesystem verdict. Repairing or formatting on top
    /// of one could destroy recoverable data.
    Failed,
}

fn classify_e2fsck(output: &std::process::Output) -> E2fsckOutcome {
    let Some(code) = output.status.code() else {
        return E2fsckOutcome::Failed;
    };
    if (code & !0b11) == 0 {
        E2fsckOutcome::Clean
    } else if (code & !0b111) == 0 {
        E2fsckOutcome::Corrupt
    } else {
        E2fsckOutcome::Failed
    }
}

fn e2fsck_exited(output: &std::process::Output) -> PreparePartitionError {
    PreparePartitionError::CommandExited {
        program: "e2fsck".to_owned(),
        status: output.status,
        stderr: stderr_snippet(&output.stderr),
    }
}

/// The deployed BusyBox does not ship `mountpoint(1)`, so the mount
/// table is consulted directly. `/proc/mounts` octal-escapes whitespace
/// and backslashes in paths (e.g. `\040` for a space), so fields are
/// decoded before comparing.
fn is_mount_point(mounts: &str, mount_point: &Path) -> bool {
    let needle = mount_point.to_string_lossy();
    mounts
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .any(|mounted| crate::mount::decode_mount_field(mounted) == needle)
}

/// Whether `mount_point` is an active mount according to
/// `/proc/mounts`.
pub fn is_path_mounted(mount_point: &Path) -> Result<bool, PreparePartitionError> {
    let mounts = read_proc_mounts()?;
    Ok(is_mount_point(&mounts, mount_point))
}

/// The current mount table from `/proc/mounts`.
pub fn read_proc_mounts() -> Result<String, PreparePartitionError> {
    std::fs::read_to_string("/proc/mounts")
        .map_err(|source| PreparePartitionError::ReadMounts { source })
}

/// Whether `mount_point` already carries the requested partition.
#[derive(Debug)]
enum MountState {
    /// Nothing is mounted at `mount_point`.
    Absent,
    /// The requested partition is already mounted at `mount_point`.
    Prepared,
}

/// Inspect `/proc/mounts` for what, if anything, occupies `mount_point`.
///
/// A line's first whitespace-separated field is the source device and
/// the second is the mount point; both may carry octal escapes. When the
/// mount point is occupied by a different device than `partition`,
/// preparation must abort rather than reformat a foreign filesystem.
fn mount_state(
    mounts: &str,
    mount_point: &Path,
    partition: &Path,
) -> Result<MountState, PreparePartitionError> {
    let needle = mount_point.to_string_lossy();
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let (Some(source), Some(mounted)) = (fields.next(), fields.next()) else {
            continue;
        };
        if crate::mount::decode_mount_field(mounted) != needle {
            continue;
        }
        let source = crate::mount::decode_mount_field(source);
        return if source == partition.to_string_lossy() {
            Ok(MountState::Prepared)
        } else {
            Err(PreparePartitionError::ForeignMount {
                mount_point: path_string(mount_point),
                requested: path_string(partition),
                mounted: source,
            })
        };
    }
    Ok(MountState::Absent)
}

/// Whether the block device `device_id` (`major:minor`) backs any mount
/// listed in `mountinfo`, comparing against the `major:minor` in each
/// line's third field. Lines that do not parse are skipped: the guard
/// acts only on entries it can positively identify.
fn is_device_id_mounted(mountinfo: &str, device_id: (u32, u32)) -> bool {
    mountinfo.lines().any(|line| {
        let Some(field) = line.split_whitespace().nth(2) else {
            return false;
        };

        let Some((major, minor)) = field.split_once(':') else {
            return false;
        };

        let (Ok(major), Ok(minor)) = (major.parse::<u32>(), minor.parse::<u32>()) else {
            return false;
        };

        (major, minor) == device_id
    })
}

/// Decode a Linux `dev_t` into `(major, minor)` (glibc encoding).
/// Decoded locally because `libc::major`/`libc::minor` have
/// platform-specific signatures that fail to compile on darwin, where
/// this Linux-only guard is still type-checked as part of the
/// workspace.
fn decode_dev_t(rdev: u64) -> (u32, u32) {
    let major = ((rdev >> 32) & 0xffff_f000) | ((rdev >> 8) & 0xfff);
    let minor = ((rdev >> 12) & 0xffff_ff00) | (rdev & 0xff);
    (
        u32::try_from(major).expect("BUG: masked to fit u32"),
        u32::try_from(minor).expect("BUG: masked to fit u32"),
    )
}

/// Whether `partition` backs any active mount listed in `mountinfo`,
/// compared by block identity (`major:minor`) so device aliases and
/// bind mounts cannot slip past. A missing node or a non-block-device
/// path cannot be mounted and passes the guard; missing-device
/// reporting belongs to the `test -b` step that runs before it. Any
/// other stat failure is propagated rather than silently read as "not
/// mounted", so a transient error on a device that does back a live
/// mount cannot let preparation fsck or reformat it.
fn is_device_mounted(partition: &Path, mountinfo: &str) -> Result<bool, PreparePartitionError> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    let metadata = match std::fs::metadata(partition) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(PreparePartitionError::StatDataPartition {
                partition: path_string(partition),
                source,
            });
        }
    };

    if !metadata.file_type().is_block_device() {
        return Ok(false);
    }

    Ok(is_device_id_mounted(
        mountinfo,
        decode_dev_t(metadata.rdev()),
    ))
}

/// The current mount table from `/proc/self/mountinfo`.
pub fn read_proc_self_mountinfo() -> Result<String, PreparePartitionError> {
    std::fs::read_to_string("/proc/self/mountinfo")
        .map_err(|source| PreparePartitionError::ReadMountInfo { source })
}

/// `mounts` and `mountinfo` are the mount tables in `/proc/mounts` and
/// `/proc/self/mountinfo` format (see [`read_proc_mounts`] and
/// [`read_proc_self_mountinfo`]), passed in so the mount inspection
/// does not depend on the environment it runs in.
pub async fn prepare_data_partition(
    runner: &impl CommandRunner,
    partition: &Path,
    mount_point: &Path,
    mounts: &str,
    mountinfo: &str,
) -> Result<(), PreparePartitionError> {
    let partition_arg = partition.to_string_lossy();
    let mount_point_arg = mount_point.to_string_lossy();

    match mount_state(mounts, mount_point, partition)? {
        MountState::Prepared => return Ok(()),
        MountState::Absent => {}
    }

    let block_device = run_command(runner, "test", &["-b", partition_arg.as_ref()]).await?;
    if !block_device.status.success() {
        return Err(PreparePartitionError::NoDataPartition {
            partition: path_string(partition),
        });
    }

    if is_device_mounted(partition, mountinfo)? {
        return Err(PreparePartitionError::PartitionMounted {
            partition: path_string(partition),
        });
    }

    let blkid = run_command(
        runner,
        "blkid",
        &["-o", "value", "-s", "TYPE", partition_arg.as_ref()],
    )
    .await?;
    if String::from_utf8_lossy(&blkid.stdout).trim().is_empty() {
        // Empty output means either an unformatted partition (safe to
        // format) or a blkid that could not examine the device. Only the
        // former may fall through to mkfs; formatting on a probe failure
        // would destroy an existing filesystem. The device ships BusyBox
        // blkid, whose exit code for "no filesystem" is not reliable, so
        // treat any error output as "could not examine" rather than trusting
        // the status. A probe killed by a signal (e.g. the OOM killer) can
        // die with empty output, so that is distinguished via the status.
        if !blkid.stderr.is_empty() || blkid.status.code().is_none() {
            return Err(PreparePartitionError::CommandExited {
                program: "blkid".to_owned(),
                status: blkid.status,
                stderr: stderr_snippet(&blkid.stderr),
            });
        }
        let mkfs = run_command(runner, "mkfs.ext4", &["-F", partition_arg.as_ref()]).await?;
        require_success("mkfs.ext4", &mkfs)?;
    }

    let preen = run_command(runner, "e2fsck", &["-p", partition_arg.as_ref()]).await?;
    match classify_e2fsck(&preen) {
        E2fsckOutcome::Clean => {}
        E2fsckOutcome::Corrupt => {
            tracing::warn!(
                partition = %partition_arg,
                "preen fsck could not repair the filesystem, escalating to e2fsck -y",
            );
            let full = run_command(runner, "e2fsck", &["-y", partition_arg.as_ref()]).await?;
            match classify_e2fsck(&full) {
                E2fsckOutcome::Clean => {
                    tracing::warn!(
                        partition = %partition_arg,
                        "filesystem needed non-preen repair; if the store misbehaves, \
                         recover with `bmc-nix-cli init --wipe` while /nix is not an \
                         active mount",
                    );
                }
                E2fsckOutcome::Corrupt => {
                    tracing::error!(
                        partition = %partition_arg,
                        "filesystem is unrecoverable even by e2fsck -y, reformatting",
                    );
                    let mkfs =
                        run_command(runner, "mkfs.ext4", &["-F", partition_arg.as_ref()]).await?;
                    require_success("mkfs.ext4", &mkfs)?;
                }
                E2fsckOutcome::Failed => return Err(e2fsck_exited(&full)),
            }
        }
        E2fsckOutcome::Failed => return Err(e2fsck_exited(&preen)),
    }

    std::fs::create_dir_all(mount_point).map_err(|source| {
        PreparePartitionError::CreateMountPoint {
            path: path_string(mount_point),
            source,
        }
    })?;

    let mount = run_command(
        runner,
        "mount",
        &[
            "-t",
            "ext4",
            partition_arg.as_ref(),
            mount_point_arg.as_ref(),
        ],
    )
    .await?;
    require_success("mount", &mount)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::os::unix::process::ExitStatusExt;
    use std::path::Path;

    use super::prepare_data_partition;
    use crate::store::CommandRunner;

    struct ScriptedRunner {
        outputs: std::sync::Mutex<VecDeque<std::process::Output>>,
        invocations: std::sync::Mutex<Vec<(String, Vec<String>)>>,
    }

    impl ScriptedRunner {
        fn new(outputs: Vec<std::process::Output>) -> Self {
            Self {
                outputs: std::sync::Mutex::new(outputs.into()),
                invocations: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn success(stdout: &[u8]) -> std::process::Output {
            Self::output(0, stdout)
        }

        fn output(code: i32, stdout: &[u8]) -> std::process::Output {
            std::process::Output {
                status: std::process::ExitStatus::from_raw(code << 8),
                stdout: stdout.to_vec(),
                stderr: Vec::new(),
            }
        }

        fn output_with_stderr(code: i32, stdout: &[u8], stderr: &[u8]) -> std::process::Output {
            std::process::Output {
                status: std::process::ExitStatus::from_raw(code << 8),
                stdout: stdout.to_vec(),
                stderr: stderr.to_vec(),
            }
        }

        fn signal(signal: i32) -> std::process::Output {
            std::process::Output {
                status: std::process::ExitStatus::from_raw(signal),
                stdout: Vec::new(),
                stderr: Vec::new(),
            }
        }

        fn invocations(&self) -> Vec<(String, Vec<String>)> {
            self.invocations
                .lock()
                .expect("BUG: invocations mutex poisoned")
                .clone()
        }
    }

    impl CommandRunner for ScriptedRunner {
        async fn run(
            &self,
            program: &str,
            args: &[&str],
        ) -> Result<std::process::Output, std::io::Error> {
            self.invocations
                .lock()
                .expect("BUG: invocations mutex poisoned")
                .push((
                    program.to_owned(),
                    args.iter().map(|arg| (*arg).to_owned()).collect(),
                ));
            self.outputs
                .lock()
                .expect("BUG: outputs mutex poisoned")
                .pop_front()
                .ok_or_else(|| std::io::Error::other("missing scripted output"))
        }

        async fn run_with_stderr_lines<F>(
            &self,
            _program: &str,
            _args: &[&str],
            _on_line: F,
        ) -> Result<std::process::Output, std::io::Error>
        where
            F: FnMut(&str) + Send,
        {
            Err(std::io::Error::other(
                "BUG: partition prep must not stream stderr",
            ))
        }
    }

    fn assert_invocations(runner: &ScriptedRunner, expected: &[(&str, &[&str])]) {
        let actual = runner.invocations();
        let expected: Vec<(String, Vec<String>)> = expected
            .iter()
            .map(|(program, args)| {
                (
                    (*program).to_owned(),
                    args.iter().map(|arg| (*arg).to_owned()).collect(),
                )
            })
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn is_mount_point_finds_mounted_path() {
        let mounts = "/dev/root / ext4 rw 0 0\n/dev/mmcblk0p4 /mnt/data ext4 rw 0 0\n";
        assert!(super::is_mount_point(mounts, Path::new("/mnt/data")));
    }

    #[test]
    fn is_mount_point_rejects_unmounted_and_prefix_paths() {
        let mounts = "/dev/root / ext4 rw 0 0\n/dev/mmcblk0p4 /mnt/data ext4 rw 0 0\n";
        assert!(!super::is_mount_point(mounts, Path::new("/mnt")));
        assert!(!super::is_mount_point(mounts, Path::new("/mnt/data/nix")));
    }

    #[test]
    fn foreign_source_at_mount_point_is_an_error() {
        let mounts = "tmpfs /mnt/data tmpfs rw 0 0\n";
        super::mount_state(mounts, Path::new("/mnt/data"), Path::new("/dev/mmcblk0p4"))
            .expect_err("BUG: a foreign source at the mount point must not read as prepared");
    }

    #[test]
    fn matching_source_at_mount_point_is_prepared() {
        let mounts = "/dev/mmcblk0p4 /mnt/data ext4 rw 0 0\n";
        assert!(matches!(
            super::mount_state(mounts, Path::new("/mnt/data"), Path::new("/dev/mmcblk0p4")),
            Ok(super::MountState::Prepared)
        ));
    }

    #[test]
    fn is_mount_point_decodes_octal_escapes() {
        let mounts = "/dev/mmcblk0p4 /mnt/data\\040disk ext4 rw 0 0\n";
        assert!(super::is_mount_point(mounts, Path::new("/mnt/data disk")));
        assert!(!super::is_mount_point(
            mounts,
            Path::new("/mnt/data\\040disk")
        ));
    }

    #[tokio::test]
    async fn prepare_data_partition_formats_unformatted_partition_then_mounts() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b""),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect("BUG: unformatted partition should be prepared");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
                ("mkfs.ext4", &["-F", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-p", "/dev/mmcblk0p4"]),
                (
                    "mount",
                    &[
                        "-t",
                        "ext4",
                        "/dev/mmcblk0p4",
                        mount_point.to_str().expect("BUG: utf8 path"),
                    ],
                ),
            ],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_fscks_formatted_partition_then_mounts() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b"ext4\n"),
            ScriptedRunner::output(1, b""),
            ScriptedRunner::success(b""),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect("BUG: formatted partition should be prepared");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-p", "/dev/mmcblk0p4"]),
                (
                    "mount",
                    &[
                        "-t",
                        "ext4",
                        "/dev/mmcblk0p4",
                        mount_point.to_str().expect("BUG: utf8 path"),
                    ],
                ),
            ],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_does_not_format_when_blkid_errors() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        // blkid prints no TYPE but reports an error: the partition may hold a
        // filesystem it simply could not read, so it must not be formatted.
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::output_with_stderr(4, b"", b"blkid: /dev/mmcblk0p4: I/O error"),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect_err("BUG: a failed blkid probe must not format the partition");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
            ],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_does_not_format_when_blkid_is_killed() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        // blkid dies on a signal (e.g. the OOM killer) with no output at
        // all: the partition was never probed, so it must not be formatted.
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            std::process::Output {
                status: std::process::ExitStatus::from_raw(libc::SIGKILL),
                stdout: Vec::new(),
                stderr: Vec::new(),
            },
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect_err("BUG: a signal-killed blkid probe must not format the partition");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
            ],
        );
    }

    #[test]
    fn is_device_id_mounted_parses_and_skips_malformed_lines() {
        let mountinfo = "36 35 179:4 / /mnt/data rw - ext4 /dev/mmcblk0p4 rw\n\
                         37 35 0:25 / /tmp rw,nosuid - tmpfs tmpfs rw\n\
                         truncated line\n\
                         38 35 not-a-device-id / /x rw - ext4 /dev/foo rw\n\
                         39 35 179:banana / /y rw - ext4 /dev/bar rw\n";
        // Both well-formed lines match despite the malformed lines
        // interleaved; a device id absent from every parsable line does
        // not.
        assert!(super::is_device_id_mounted(mountinfo, (179, 4)));
        assert!(super::is_device_id_mounted(mountinfo, (0, 25)));
        assert!(!super::is_device_id_mounted(mountinfo, (179, 5)));
    }

    #[test]
    fn decode_dev_t_splits_major_minor() {
        // Linux encodes major in bits 8-19 and 44-63, minor in bits 0-7
        // and 20-43.
        assert_eq!(super::decode_dev_t((179 << 8) | 4), (179, 4));
        assert_eq!(super::decode_dev_t((179 << 8) | (1 << 20)), (179, 256));
    }

    #[test]
    fn is_device_id_mounted_matches_by_block_identity() {
        // A bind mount of a subdirectory reports the same major:minor as
        // the origin mount, so identity comparison catches it while any
        // source-string comparison could not.
        let mountinfo = "40 30 179:4 /nix /nix rw - ext4 /dev/mmcblk0p4 rw\n";
        assert!(super::is_device_id_mounted(mountinfo, (179, 4)));
        assert!(!super::is_device_id_mounted(mountinfo, (179, 5)));
    }

    #[test]
    fn is_device_mounted_passes_nonexistent_path() {
        // A missing node cannot be mounted; missing-device reporting
        // belongs to the `test -b` step.
        let mountinfo = "40 30 179:4 / /mnt/data rw - ext4 /dev/mmcblk0p4 rw\n";
        assert!(
            !super::is_device_mounted(Path::new("/nonexistent/device"), mountinfo)
                .expect("BUG: a missing node must read as not mounted")
        );
    }

    #[test]
    fn is_device_mounted_passes_non_block_device() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let file = tmp.path().join("not-a-device");
        std::fs::write(&file, b"").expect("BUG: write temp file");
        let mountinfo = "40 30 179:4 / /mnt/data rw - ext4 /dev/mmcblk0p4 rw\n";
        assert!(
            !super::is_device_mounted(&file, mountinfo)
                .expect("BUG: a non-block-device path must read as not mounted")
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_fast_paths_when_partition_already_mounted() {
        // The requested partition already backs the mount point: the
        // normal already-prepared case must not run any command.
        let runner = ScriptedRunner::new(vec![]);
        let mounts = "/dev/mmcblk0p4 /mnt/data ext4 rw 0 0\n";

        prepare_data_partition(
            &runner,
            Path::new("/dev/mmcblk0p4"),
            Path::new("/mnt/data"),
            mounts,
            "",
        )
        .await
        .expect("BUG: already-mounted partition should fast-path");

        assert_invocations(&runner, &[]);
    }

    #[tokio::test]
    async fn prepare_data_partition_rejects_foreign_mount_before_any_command() {
        // A different device at the mount point must abort before any
        // command runs; reformatting would destroy a foreign filesystem.
        let runner = ScriptedRunner::new(vec![]);
        let mounts = "tmpfs /mnt/data tmpfs rw 0 0\n";

        let err = prepare_data_partition(
            &runner,
            Path::new("/dev/mmcblk0p4"),
            Path::new("/mnt/data"),
            mounts,
            "",
        )
        .await
        .expect_err("BUG: foreign mount must abort preparation");

        assert!(matches!(
            err,
            super::PreparePartitionError::ForeignMount { .. }
        ));
        assert_invocations(&runner, &[]);
    }

    #[tokio::test]
    async fn prepare_data_partition_accepts_preen_reboot_advice() {
        // Exit 2 = corrected, reboot recommended. The partition is
        // verified unmounted, so the advice does not apply and the fs is
        // safe to mount without escalation.
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b"ext4\n"),
            ScriptedRunner::output(2, b""),
            ScriptedRunner::success(b""),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect("BUG: preen exit 2 should mount");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-p", "/dev/mmcblk0p4"]),
                (
                    "mount",
                    &[
                        "-t",
                        "ext4",
                        "/dev/mmcblk0p4",
                        mount_point.to_str().expect("BUG: utf8 path"),
                    ],
                ),
            ],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_accepts_preen_exit_3() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b"ext4\n"),
            ScriptedRunner::output(3, b""),
            ScriptedRunner::success(b""),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect("BUG: preen exit 3 should mount");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-p", "/dev/mmcblk0p4"]),
                (
                    "mount",
                    &[
                        "-t",
                        "ext4",
                        "/dev/mmcblk0p4",
                        mount_point.to_str().expect("BUG: utf8 path"),
                    ],
                ),
            ],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_escalates_when_preen_cannot_repair() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b"ext4\n"),
            ScriptedRunner::output(6, b""),
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b""),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect("BUG: escalated repair should mount");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-p", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-y", "/dev/mmcblk0p4"]),
                (
                    "mount",
                    &[
                        "-t",
                        "ext4",
                        "/dev/mmcblk0p4",
                        mount_point.to_str().expect("BUG: utf8 path"),
                    ],
                ),
            ],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_escalated_repair_mounts() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b"ext4\n"),
            ScriptedRunner::output(4, b""),
            ScriptedRunner::output(1, b""),
            ScriptedRunner::success(b""),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect("BUG: -y exit 1 should mount");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-p", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-y", "/dev/mmcblk0p4"]),
                (
                    "mount",
                    &[
                        "-t",
                        "ext4",
                        "/dev/mmcblk0p4",
                        mount_point.to_str().expect("BUG: utf8 path"),
                    ],
                ),
            ],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_escalated_repair_with_reboot_advice_mounts() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b"ext4\n"),
            ScriptedRunner::output(4, b""),
            ScriptedRunner::output(3, b""),
            ScriptedRunner::success(b""),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect("BUG: -y exit 3 should mount");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-p", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-y", "/dev/mmcblk0p4"]),
                (
                    "mount",
                    &[
                        "-t",
                        "ext4",
                        "/dev/mmcblk0p4",
                        mount_point.to_str().expect("BUG: utf8 path"),
                    ],
                ),
            ],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_reformats_unrecoverable_filesystem() {
        // -y exit 6 = corrected what it could (2) but errors remain (4):
        // the fs is unrecoverable, rebuild it. A fresh fs needs no fsck,
        // so mkfs goes straight to mount.
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b"ext4\n"),
            ScriptedRunner::output(4, b""),
            ScriptedRunner::output(6, b""),
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b""),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect("BUG: unrecoverable fs should reformat and mount");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-p", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-y", "/dev/mmcblk0p4"]),
                ("mkfs.ext4", &["-F", "/dev/mmcblk0p4"]),
                (
                    "mount",
                    &[
                        "-t",
                        "ext4",
                        "/dev/mmcblk0p4",
                        mount_point.to_str().expect("BUG: utf8 path"),
                    ],
                ),
            ],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_preen_operational_error_does_not_escalate() {
        // Bit 8 is a tool/device failure, not a filesystem verdict:
        // answering yes to repairs on a device in that state could turn a
        // transient problem into real damage.
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b"ext4\n"),
            ScriptedRunner::output(8, b""),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect_err("BUG: operational preen failure must not escalate");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-p", "/dev/mmcblk0p4"]),
            ],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_preen_unknown_bit_does_not_escalate() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b"ext4\n"),
            ScriptedRunner::output(64, b""),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect_err("BUG: unknown exit bit must not escalate");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-p", "/dev/mmcblk0p4"]),
            ],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_preen_signal_death_does_not_escalate() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b"ext4\n"),
            ScriptedRunner::signal(9),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect_err("BUG: signal death must not escalate");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-p", "/dev/mmcblk0p4"]),
            ],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_preen_spawn_error_does_not_escalate() {
        // The scripted queue runs dry at the -p invocation, which the
        // runner surfaces as a spawn io::Error.
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b"ext4\n"),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect_err("BUG: preen spawn error must not escalate");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-p", "/dev/mmcblk0p4"]),
            ],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_full_fsck_operational_error_does_not_reformat() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b"ext4\n"),
            ScriptedRunner::output(4, b""),
            ScriptedRunner::output(8, b""),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect_err("BUG: -y operational failure must not reformat");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-p", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-y", "/dev/mmcblk0p4"]),
            ],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_combined_corruption_and_operational_bits_do_not_reformat() {
        // 12 = 8 | 4: the corruption bit is meaningless when the run
        // itself failed; formatting on it could wipe a recoverable fs.
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b"ext4\n"),
            ScriptedRunner::output(4, b""),
            ScriptedRunner::output(12, b""),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect_err("BUG: -y exit 12 must not reformat");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-p", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-y", "/dev/mmcblk0p4"]),
            ],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_full_fsck_spawn_error_does_not_reformat() {
        // The scripted queue runs dry at the -y invocation, which the
        // runner surfaces as a spawn io::Error.
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b"ext4\n"),
            ScriptedRunner::output(4, b""),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point, "", "")
            .await
            .expect_err("BUG: -y spawn error must not reformat");

        assert_invocations(
            &runner,
            &[
                ("test", &["-b", "/dev/mmcblk0p4"]),
                ("blkid", &["-o", "value", "-s", "TYPE", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-p", "/dev/mmcblk0p4"]),
                ("e2fsck", &["-y", "/dev/mmcblk0p4"]),
            ],
        );
    }
}
