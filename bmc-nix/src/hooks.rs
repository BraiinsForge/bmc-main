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

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

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
    #[error(transparent)]
    ListEntries(#[from] ExecutableEntriesError),
}

/// Errors that can occur while listing the executable entries of a directory.
#[derive(Debug, thiserror::Error)]
pub enum ExecutableEntriesError {
    #[error("failed to read directory '{path}': {source}")]
    ReadDir {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to stat entry '{path}': {source}")]
    Stat {
        path: String,
        source: std::io::Error,
    },
    #[error("entry '{path}' is not a regular executable file")]
    NotExecutable { path: String },
    #[error("entry '{path}' has a non-UTF-8 filename")]
    NonUtf8Name { path: String },
}

/// List a directory's entries as `(name, path)` pairs sorted by UTF-8 name,
/// keeping only entries whose resolved metadata is a regular executable file.
///
/// Symlinks are followed via [`std::fs::metadata`] (unlike
/// [`std::fs::DirEntry::metadata`], which stats the link itself), so a link
/// to an executable is honored. Any entry that is not a regular executable
/// file — a subdirectory, a non-executable file, or an entry whose name is
/// not valid UTF-8 — is a hard error naming the offending path.
pub fn executable_entries(dir: &Path) -> Result<Vec<(String, PathBuf)>, ExecutableEntriesError> {
    let read_dir = std::fs::read_dir(dir).map_err(|source| ExecutableEntriesError::ReadDir {
        path: dir.display().to_string(),
        source,
    })?;

    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    for entry in read_dir {
        let entry = entry.map_err(|source| ExecutableEntriesError::ReadDir {
            path: dir.display().to_string(),
            source,
        })?;
        let path = entry.path();

        let metadata = std::fs::metadata(&path).map_err(|source| ExecutableEntriesError::Stat {
            path: path.display().to_string(),
            source,
        })?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return Err(ExecutableEntriesError::NotExecutable {
                path: path.display().to_string(),
            });
        }

        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| ExecutableEntriesError::NonUtf8Name {
                path: path.display().to_string(),
            })?
            .to_owned();

        entries.push((name, path));
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));
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
/// Every entry in the hooks directory must be a regular executable file (symlinks are
/// followed); a subdirectory, a non-executable file, or a non-UTF-8 name is a hard error
/// naming the offending path before any hook runs. Hooks are executed in lexicographic
/// order by filename. If any hook exits with a non-zero exit code, execution stops and an
/// error is returned.
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

    let entries = executable_entries(&hooks_dir)?;

    if entries.is_empty() {
        debug!(?hooks_dir, "hooks directory is empty, skipping");
        return Ok(());
    }

    for (hook_name, hook_path) in entries {
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

    fn create_file_with_mode(dir: &Path, name: &str, mode: u32) -> PathBuf {
        use std::io::Write;

        let path = dir.join(name);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(&path)
            .expect("BUG: create file");
        file.write_all(b"#!/bin/sh\nexit 0\n")
            .expect("BUG: write file");
        drop(file);
        path
    }

    #[test]
    fn executable_entries_sorted_and_symlinks_followed() {
        let tmp = tempfile::tempdir().expect("BUG: create tempdir");
        let dir = tmp.path().join("scripts");
        std::fs::create_dir_all(&dir).expect("BUG: create scripts dir");

        // A plain executable file.
        create_file_with_mode(&dir, "b-real", 0o755);

        // An executable target reached through a symlink; std::fs::metadata
        // follows the link, so it must be listed.
        let target = tmp.path().join("target-exec");
        create_file_with_mode(tmp.path(), "target-exec", 0o755);
        std::os::unix::fs::symlink(&target, dir.join("a-link")).expect("BUG: symlink");

        let entries = executable_entries(&dir).expect("BUG: listing should succeed");
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, vec!["a-link", "b-real"]);
    }

    #[test]
    fn executable_entries_rejects_subdir() {
        let tmp = tempfile::tempdir().expect("BUG: create tempdir");
        let dir = tmp.path().join("scripts");
        std::fs::create_dir_all(dir.join("nested")).expect("BUG: create nested dir");

        let err = executable_entries(&dir).expect_err("BUG: subdir must be rejected");
        assert!(
            err.to_string().contains("nested"),
            "error should name the offending entry, got: {err}"
        );
    }

    #[test]
    fn executable_entries_rejects_non_executable() {
        let tmp = tempfile::tempdir().expect("BUG: create tempdir");
        let dir = tmp.path().join("scripts");
        std::fs::create_dir_all(&dir).expect("BUG: create scripts dir");

        create_file_with_mode(&dir, "10-stray", 0o644);

        let err = executable_entries(&dir).expect_err("BUG: non-executable must be rejected");
        assert!(
            err.to_string().contains("10-stray"),
            "error should name the offending entry, got: {err}"
        );
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
            | RunHooksError::ListEntries(..)) => {
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
            | RunHooksError::ListEntries(..)) => {
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
            | RunHooksError::ListEntries(..)) => {
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
