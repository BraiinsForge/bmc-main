// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::Path;

use tracing::{debug, info};

/// Errors that can occur when running hook scripts.
#[derive(Debug, thiserror::Error)]
pub enum RunHooksError {
    #[error("hook '{hook}' failed with exit code {exit_code}: {output}")]
    HookFailed {
        hook: String,
        exit_code: i32,
        output: String,
    },
    #[error("hook '{hook}' was terminated by signal: {output}")]
    HookSignaled { hook: String, output: String },
    #[error("failed to execute hook '{hook}': {source}")]
    Execute {
        hook: String,
        source: std::io::Error,
    },
    #[error("failed to read hooks directory '{path}': {source}")]
    ReadDir {
        path: String,
        source: std::io::Error,
    },
}

/// Read directory entries, filter to files only, and sort lexicographically by filename.
fn sorted_dir_entries(dir: &Path) -> Result<Vec<std::fs::DirEntry>, std::io::Error> {
    let mut entries: Vec<std::fs::DirEntry> = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        // Follow symlinks: use std::fs::metadata which resolves symlinks,
        // unlike DirEntry::file_type which returns the symlink type itself.
        let metadata = std::fs::metadata(entry.path())?;
        if metadata.is_file() {
            entries.push(entry);
        }
    }

    entries.sort_by_key(std::fs::DirEntry::file_name);
    Ok(entries)
}

/// Discover and execute hook scripts in lexicographic order from a profile's hooks directory.
///
/// Hooks are executable files located in `new_gen_path/hooks_dir_name/`. Each hook is executed
/// as a subprocess with the environment variable `PROFILE_NEW_GENERATION` set to `new_gen_path`.
///
/// When `hooks_override_path` is `Some`, hooks are discovered and executed from that path
/// instead of from the profile. This is needed for cross-compilation bootstrap: during init
/// tarball builds on x86_64, the profile contains ARM hooks that cannot run natively.
///
/// If the hooks directory does not exist or is empty, this is a no-op and returns `Ok(())`.
/// Hooks are executed in lexicographic order by filename. If any hook exits with a non-zero
/// exit code, execution stops and an error is returned.
pub async fn run_hooks(
    new_gen_path: &Path,
    hooks_dir_name: &str,
    hooks_override_path: Option<&Path>,
) -> Result<(), RunHooksError> {
    let hooks_dir = match hooks_override_path {
        Some(override_path) => override_path.to_path_buf(),
        None => new_gen_path.join(hooks_dir_name),
    };

    if !hooks_dir.exists() {
        debug!(?hooks_dir, "hooks directory does not exist, skipping");
        return Ok(());
    }

    let entries = sorted_dir_entries(&hooks_dir).map_err(|source| RunHooksError::ReadDir {
        path: hooks_dir.display().to_string(),
        source,
    })?;

    if entries.is_empty() {
        debug!(?hooks_dir, "hooks directory is empty, skipping");
        return Ok(());
    }

    for entry in entries {
        let hook_name = entry.file_name().to_string_lossy().to_string();
        let hook_path = entry.path();

        info!(?hook_path, "executing hook");

        let mut command = tokio::process::Command::new(&hook_path);
        command.env("PROFILE_NEW_GENERATION", new_gen_path);
        let output = crate::store::output_bounded(command)
            .await
            .map_err(|source| RunHooksError::Execute {
                hook: hook_name.clone(),
                source,
            })?;

        let snippet = if output.stderr.is_empty() {
            crate::store::stderr_snippet(&output.stdout)
        } else {
            crate::store::stderr_snippet(&output.stderr)
        };

        if !output.status.success() {
            tracing::warn!(?hook_path, status = ?output.status, output = %snippet, "hook failed");
            match output.status.code() {
                Some(exit_code) => {
                    return Err(RunHooksError::HookFailed {
                        hook: hook_name,
                        exit_code,
                        output: snippet,
                    });
                }
                None => {
                    return Err(RunHooksError::HookSignaled {
                        hook: hook_name,
                        output: snippet,
                    });
                }
            }
        }

        debug!(?hook_path, output = %snippet, "hook completed successfully");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::OpenOptionsExt;

    use serial_test::serial;

    use super::*;

    fn create_hook_script(dir: &Path, name: &str, content: &str) {
        use std::io::Write;

        let script = dir.join(name);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o755)
            .open(&script)
            .expect("BUG: create hook script");
        file.write_all(content.as_bytes())
            .expect("BUG: write hook script");
        file.sync_all().expect("BUG: sync hook script");
        // Explicit drop to ensure the fd is closed before exec
        drop(file);
    }

    #[tokio::test]
    #[serial]
    async fn run_hooks_executes_in_order() {
        let tmp = tempfile::tempdir().expect("BUG: create tempdir");
        let gen_path = tmp.path().join("gen-1");
        let hooks_dir = gen_path.join("hooks");
        std::fs::create_dir_all(&hooks_dir).expect("BUG: create hooks dir");

        let log_file = tmp.path().join("log.txt");

        create_hook_script(
            &hooks_dir,
            "01-first",
            &format!("#!/bin/sh\necho first >> {}\n", log_file.display()),
        );
        create_hook_script(
            &hooks_dir,
            "02-second",
            &format!("#!/bin/sh\necho second >> {}\n", log_file.display()),
        );

        run_hooks(&gen_path, "hooks", None)
            .await
            .expect("BUG: run_hooks should succeed");

        let log_content = std::fs::read_to_string(&log_file).expect("BUG: read log file");
        let lines: Vec<&str> = log_content.trim().lines().collect();
        assert_eq!(lines, vec!["first", "second"]);
    }

    #[tokio::test]
    #[serial]
    async fn empty_hooks_dir_succeeds() {
        let tmp = tempfile::tempdir().expect("BUG: create tempdir");
        let gen_path = tmp.path().join("gen-1");
        let hooks_dir = gen_path.join("hooks");
        std::fs::create_dir_all(&hooks_dir).expect("BUG: create hooks dir");

        let result = run_hooks(&gen_path, "hooks", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[serial]
    async fn missing_hooks_dir_succeeds() {
        let tmp = tempfile::tempdir().expect("BUG: create tempdir");
        let gen_path = tmp.path().join("gen-1");
        // Do not create hooks dir
        std::fs::create_dir_all(&gen_path).expect("BUG: create gen dir");

        let result = run_hooks(&gen_path, "hooks", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[serial]
    async fn hook_failure_propagates_error() {
        let tmp = tempfile::tempdir().expect("BUG: create tempdir");
        let gen_path = tmp.path().join("gen-1");
        let hooks_dir = gen_path.join("hooks");
        std::fs::create_dir_all(&hooks_dir).expect("BUG: create hooks dir");

        create_hook_script(&hooks_dir, "01-fail", "#!/bin/sh\nexit 42\n");

        let result = run_hooks(&gen_path, "hooks", None).await;
        assert!(result.is_err());

        let err = result.expect_err("BUG: failing hook must produce an error");
        match err {
            RunHooksError::HookFailed {
                hook, exit_code, ..
            } => {
                assert_eq!(hook, "01-fail");
                assert_eq!(exit_code, 42);
            }
            other @ (RunHooksError::HookSignaled { .. }
            | RunHooksError::Execute { .. }
            | RunHooksError::ReadDir { .. }) => {
                panic!("expected HookFailed, got: {other}")
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn hook_failure_carries_stderr_snippet() {
        let tmp = tempfile::tempdir().expect("BUG: create tempdir");
        let gen_path = tmp.path().join("gen-1");
        let hooks_dir = gen_path.join("hooks");
        std::fs::create_dir_all(&hooks_dir).expect("BUG: create hooks dir");

        create_hook_script(
            &hooks_dir,
            "01-fail",
            "#!/bin/sh\necho broken manifest >&2\nexit 3\n",
        );

        let err = run_hooks(&gen_path, "hooks", None)
            .await
            .expect_err("BUG: failing hook must error");
        match err {
            RunHooksError::HookFailed { output, .. } => {
                assert!(output.contains("broken manifest"), "got output: {output:?}");
            }
            other @ (RunHooksError::HookSignaled { .. }
            | RunHooksError::Execute { .. }
            | RunHooksError::ReadDir { .. }) => {
                panic!("expected HookFailed, got: {other}")
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn hook_signal_termination_is_captured() {
        let tmp = tempfile::tempdir().expect("BUG: create tempdir");
        let gen_path = tmp.path().join("gen-1");
        let hooks_dir = gen_path.join("hooks");
        std::fs::create_dir_all(&hooks_dir).expect("BUG: create hooks dir");

        create_hook_script(
            &hooks_dir,
            "01-signal",
            "#!/bin/sh\necho dying >&2\nkill -9 $$\n",
        );

        let err = run_hooks(&gen_path, "hooks", None)
            .await
            .expect_err("BUG: signaled hook must error");
        assert!(
            err.to_string().contains("dying"),
            "display must carry the output, got: {err}"
        );
        match err {
            RunHooksError::HookSignaled { output, .. } => {
                assert!(output.contains("dying"), "got output: {output:?}");
            }
            other @ (RunHooksError::HookFailed { .. }
            | RunHooksError::Execute { .. }
            | RunHooksError::ReadDir { .. }) => {
                panic!("expected HookSignaled, got: {other}")
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn hooks_receive_generation_env() {
        let tmp = tempfile::tempdir().expect("BUG: create tempdir");
        let gen_path = tmp.path().join("gen-1");
        let hooks_dir = gen_path.join("hooks");
        std::fs::create_dir_all(&hooks_dir).expect("BUG: create hooks dir");

        let env_file = tmp.path().join("env_value.txt");

        create_hook_script(
            &hooks_dir,
            "01-check-env",
            &format!(
                "#!/bin/sh\necho \"$PROFILE_NEW_GENERATION\" > {}\n",
                env_file.display()
            ),
        );

        run_hooks(&gen_path, "hooks", None)
            .await
            .expect("BUG: run_hooks should succeed");

        let env_content = std::fs::read_to_string(&env_file).expect("BUG: read env file");
        assert_eq!(
            env_content.trim(),
            gen_path
                .to_str()
                .expect("BUG: gen_path should be valid UTF-8")
        );
    }
}
