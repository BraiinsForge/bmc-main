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

fn require_e2fsck_success(output: &std::process::Output) -> Result<(), PreparePartitionError> {
    if matches!(output.status.code(), Some(0 | 1)) {
        return Ok(());
    }

    Err(PreparePartitionError::CommandExited {
        program: "e2fsck".to_owned(),
        status: output.status,
        stderr: stderr_snippet(&output.stderr),
    })
}

pub async fn prepare_data_partition(
    runner: &impl CommandRunner,
    partition: &Path,
    mount_point: &Path,
) -> Result<(), PreparePartitionError> {
    let partition_arg = partition.to_string_lossy();
    let mount_point_arg = mount_point.to_string_lossy();

    let mountpoint = run_command(runner, "mountpoint", &["-q", mount_point_arg.as_ref()]).await?;
    if mountpoint.status.success() {
        return Ok(());
    }

    let block_device = run_command(runner, "test", &["-b", partition_arg.as_ref()]).await?;
    if !block_device.status.success() {
        return Err(PreparePartitionError::NoDataPartition {
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
        // the status.
        if !blkid.stderr.is_empty() {
            return Err(PreparePartitionError::CommandExited {
                program: "blkid".to_owned(),
                status: blkid.status,
                stderr: stderr_snippet(&blkid.stderr),
            });
        }
        let mkfs = run_command(runner, "mkfs.ext4", &["-F", partition_arg.as_ref()]).await?;
        require_success("mkfs.ext4", &mkfs)?;
    }

    let e2fsck = run_command(runner, "e2fsck", &["-p", partition_arg.as_ref()]).await?;
    require_e2fsck_success(&e2fsck)?;

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

        fn failure() -> std::process::Output {
            Self::output(1, b"")
        }

        fn output(code: i32, stdout: &[u8]) -> std::process::Output {
            std::process::Output {
                status: std::process::ExitStatus::from_raw(code << 8),
                stdout: stdout.to_vec(),
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

    #[tokio::test]
    async fn prepare_data_partition_returns_when_mount_point_is_already_mounted() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let runner = ScriptedRunner::new(vec![ScriptedRunner::success(b"")]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), tmp.path())
            .await
            .expect("BUG: mounted partition should be accepted");

        assert_invocations(
            &runner,
            &[(
                "mountpoint",
                &["-q", tmp.path().to_str().expect("BUG: utf8 path")],
            )],
        );
    }

    #[tokio::test]
    async fn prepare_data_partition_formats_unformatted_partition_then_mounts() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let mount_point = tmp.path().join("data");
        let runner = ScriptedRunner::new(vec![
            ScriptedRunner::failure(),
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b""),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point)
            .await
            .expect("BUG: unformatted partition should be prepared");

        assert_invocations(
            &runner,
            &[
                (
                    "mountpoint",
                    &["-q", mount_point.to_str().expect("BUG: utf8 path")],
                ),
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
            ScriptedRunner::failure(),
            ScriptedRunner::success(b""),
            ScriptedRunner::success(b"ext4\n"),
            ScriptedRunner::output(1, b""),
            ScriptedRunner::success(b""),
        ]);

        prepare_data_partition(&runner, Path::new("/dev/mmcblk0p4"), &mount_point)
            .await
            .expect("BUG: formatted partition should be prepared");

        assert_invocations(
            &runner,
            &[
                (
                    "mountpoint",
                    &["-q", mount_point.to_str().expect("BUG: utf8 path")],
                ),
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
}
