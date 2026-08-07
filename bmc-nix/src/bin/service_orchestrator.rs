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

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use clap::Parser;
use file_rotate::compression::Compression;
use file_rotate::suffix::AppendCount;
use file_rotate::{ContentLimit, FileRotate};
use serde_json::json;
use tracing::{debug, error, info, warn};
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use bmc_nix::profile;
use bmc_nix::service_orchestrator::{
    ActivationCompletion, PlannedAction, ServiceStatus, UPGRADED_SERVICE_MARKER_DIR,
    UpgradeMarkerAction, build_action_plan, compare_generation_services, discover_generation,
    evaluate_activation_completion, parse_rcd_link_name, publish_upgraded_service_marker,
};

const LOG_FILE: &str = "/var/log/nix-orchestrator/nix-orchestrator.log";

/// Rotate the log file after crossing this threshold.
const LOG_ROTATE_THRESHOLD: usize = 512 * 1024;

/// Keep this number of old compressed files.
const LOG_ROTATE_FILES_KEEP: usize = 4;

#[derive(Debug, Parser)]
#[command(name = "bmc-nix-service-orchestrator")]
struct Args {
    // Empty path is a sentinel for "no previous generation" (first
    // activation). Clap's default PathBuf parser rejects empty values, so
    // use a passthrough parser here.
    #[arg(long, value_parser = parse_possibly_empty_path)]
    old_generation: PathBuf,

    #[arg(long)]
    new_generation: PathBuf,

    #[arg(long)]
    current_link: PathBuf,

    #[arg(long)]
    instance_name: String,

    #[arg(long)]
    timeout_seconds: usize,
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "clap's value_parser signature requires Result"
)]
fn parse_possibly_empty_path(value: &str) -> Result<PathBuf, String> {
    Ok(PathBuf::from(value))
}

fn init_logging() {
    let writer = FileRotate::new(
        LOG_FILE,
        AppendCount::new(LOG_ROTATE_FILES_KEEP),
        ContentLimit::BytesSurpassed(LOG_ROTATE_THRESHOLD),
        Compression::OnRotate(0),
        None,
    );

    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(Mutex::new(writer))
        .with_filter(tracing_subscriber::filter::LevelFilter::DEBUG);

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .with_filter(tracing_subscriber::filter::LevelFilter::INFO);

    tracing_subscriber::registry()
        .with(file_layer)
        .with(stderr_layer)
        .init();
}

fn main() -> anyhow::Result<()> {
    init_logging();

    let args = Args::parse();
    info!(
        old_generation = %args.old_generation.display(),
        new_generation = %args.new_generation.display(),
        current_link = %args.current_link.display(),
        timeout_seconds = args.timeout_seconds,
        "service orchestrator started"
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;

    let result = runtime.block_on(run(args));
    if let Err(ref err) = result {
        error!("service orchestrator failed: {err:#}");
    } else {
        info!("service orchestrator finished successfully");
    }
    result
}

async fn run(args: Args) -> anyhow::Result<()> {
    let orchestration_result = async {
        let deadline = Instant::now() + Duration::from_secs(args.timeout_seconds as u64);

        info!(
            old = %args.old_generation.display(),
            new = %args.new_generation.display(),
            "discovering generations"
        );
        let old = discover_generation(&args.old_generation).with_context(|| {
            format!(
                "failed to discover old generation '{}'",
                args.old_generation.display()
            )
        })?;
        let new = discover_generation(&args.new_generation).with_context(|| {
            format!(
                "failed to discover new generation '{}'",
                args.new_generation.display()
            )
        })?;

        info!(
            old_services = old.init_services.len(),
            new_services = new.init_services.len(),
            "discovered generation services"
        );

        let changes = compare_generation_services(&old, &new);
        log_service_changes(&changes);

        let profile_dir = args
            .current_link
            .parent()
            .context("current link must have a parent directory")?;

        info!(
            profile_dir = %profile_dir.display(),
            "waiting for profile lock"
        );
        let _profile_lock = wait_for_profile_lock(profile_dir, deadline).await?;
        info!("profile lock acquired");

        info!(
            current_link = %args.current_link.display(),
            new_generation = %args.new_generation.display(),
            "verifying current link points to new generation"
        );
        verify_current_generation(&args.current_link, &args.new_generation)?;

        let statuses = collect_upgrade_statuses(&changes)?;
        let registered =
            collect_live_registrations(Path::new("/etc/rc.d"), Path::new("/etc/init.d"));
        info!(
            count = registered.len(),
            "collected live start registrations"
        );
        let actions = build_action_plan(
            &changes,
            &statuses,
            &registered,
            &old.stop_order,
            &new.start_order,
        );
        log_action_plan(&actions);
        execute_action_plan(&actions, Path::new(UPGRADED_SERVICE_MARKER_DIR));
        Ok(())
    }
    .await;

    let cleanup_result = delete_transient_service(&args.instance_name);
    match (orchestration_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
        (Err(error), Err(cleanup_error)) => Err(error.context(format!(
            "transient service cleanup also failed: {cleanup_error:#}"
        ))),
    }
}

fn log_service_changes(changes: &bmc_nix::service_orchestrator::ServiceChanges) {
    for change in &changes.new {
        info!(service = %change.name, "detected new service");
    }
    for change in &changes.removed {
        info!(service = %change.name, "detected removed service");
    }
    for change in &changes.upgraded {
        info!(service = %change.name, "detected upgraded service");
    }
    for change in &changes.unchanged {
        debug!(service = %change.name, "unchanged service");
    }
    if changes.new.is_empty() && changes.removed.is_empty() && changes.upgraded.is_empty() {
        info!("no service changes detected");
    }
}

fn log_action_plan(actions: &[bmc_nix::service_orchestrator::PlannedAction]) {
    if actions.is_empty() {
        info!("action plan is empty, nothing to execute");
        return;
    }
    info!(count = actions.len(), "action plan built");
    for (i, action) in actions.iter().enumerate() {
        info!(
            step = i + 1,
            service = %action.service,
            action = %action.action,
            command = %action.command_path.display(),
            "planned action"
        );
    }
}

async fn wait_for_profile_lock(
    profile_dir: &Path,
    deadline: Instant,
) -> anyhow::Result<profile::ProfileLock> {
    let remaining = remaining_budget(deadline).with_context(|| {
        format!(
            "timed out before lock attempt for '{}'",
            profile_dir.display()
        )
    })?;
    let lock = profile::lock_profile_with_timeout(profile_dir, remaining)
        .await
        .with_context(|| {
            format!(
                "failed to lock profile directory '{}'",
                profile_dir.display()
            )
        })?;

    lock.with_context(|| {
        format!(
            "timed out waiting for profile directory lock '{}'",
            profile_dir.display()
        )
    })
}

fn verify_current_generation(current_link: &Path, new_generation: &Path) -> anyhow::Result<()> {
    let current_target = read_current_link_target(current_link)
        .with_context(|| format!("failed to read current link '{}'", current_link.display()))?;

    if evaluate_activation_completion(current_target, new_generation)
        == ActivationCompletion::Succeeded
    {
        info!(current_link = %current_link.display(), "current profile points to new generation");
        Ok(())
    } else {
        anyhow::bail!(
            "current profile link '{}' does not point to '{}'",
            current_link.display(),
            new_generation.display()
        );
    }
}

fn remaining_budget(deadline: Instant) -> Option<Duration> {
    deadline.checked_duration_since(Instant::now())
}

fn read_current_link_target(current_link: &Path) -> std::io::Result<Option<PathBuf>> {
    match std::fs::read_link(current_link) {
        Ok(target) if target.is_absolute() => Ok(Some(target)),
        Ok(target) => Ok(Some(
            current_link
                .parent()
                .expect("BUG: current link should always have a parent")
                .join(target),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn collect_upgrade_statuses(
    changes: &bmc_nix::service_orchestrator::ServiceChanges,
) -> anyhow::Result<BTreeMap<String, ServiceStatus>> {
    let mut statuses = BTreeMap::new();
    for change in &changes.upgraded {
        let status = read_active_service_status(&change.name)
            .with_context(|| format!("failed to get service status for '{}'", change.name))?;
        info!(service = %change.name, ?status, "checked upgrade candidate status");
        statuses.insert(change.name.clone(), status);
    }
    Ok(statuses)
}

/// Services rc could have started at boot: live `S` links that resolve
/// to the live init script. `K` links, dangling links, and links naming
/// another script do not count — rc starts nothing through them.
fn collect_live_registrations(rcd_dir: &Path, init_d_dir: &Path) -> BTreeSet<String> {
    let mut registered = BTreeSet::new();
    let entries = match std::fs::read_dir(rcd_dir) {
        Ok(entries) => entries,
        Err(error) => {
            warn!(
                rcd_dir = %rcd_dir.display(),
                %error,
                "cannot read live rc.d, treating every service as unregistered"
            );
            return registered;
        }
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(link_name) = file_name.to_str() else {
            continue;
        };
        if !link_name.starts_with('S') {
            continue;
        }
        let Some((service, _priority)) = parse_rcd_link_name(link_name) else {
            continue;
        };
        let Ok(link_target) = std::fs::canonicalize(entry.path()) else {
            continue;
        };
        let Ok(expected_target) = std::fs::canonicalize(init_d_dir.join(&service)) else {
            continue;
        };
        if link_target == expected_target {
            registered.insert(service);
        }
    }
    registered
}

fn read_active_service_status(service: &str) -> anyhow::Result<ServiceStatus> {
    let command_path = PathBuf::from("/etc/init.d").join(service);
    let status = Command::new(&command_path)
        .arg("status")
        .status()
        .with_context(|| format!("failed to run '{} status'", command_path.display()))?;

    Ok(match status.code() {
        Some(0) => ServiceStatus::Running,
        Some(_) => ServiceStatus::Stopped,
        None => ServiceStatus::Unknown,
    })
}

/// Marker write failure costs the service its restart reason, not the activation.
fn publish_service_upgrade_marker(marker_dir: &Path, service: &str) -> PathBuf {
    let marker = marker_dir.join(service);
    match publish_upgraded_service_marker(&marker) {
        Ok(()) => {
            info!(service, marker = %marker.display(), "published upgraded service marker");
        }
        Err(error) => warn!(
            service,
            marker = %marker.display(),
            %error,
            "failed to publish upgraded service marker"
        ),
    }

    marker
}

fn remove_failed_service_upgrade_marker(service: &str, marker: &Path) {
    match std::fs::remove_file(marker) {
        Ok(()) => info!(
            service,
            marker = %marker.display(),
            "removed upgraded service marker after action failure"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => warn!(
            service,
            marker = %marker.display(),
            %error,
            "failed to remove upgraded service marker after action failure"
        ),
    }
}

fn execute_action_plan(actions: &[PlannedAction], marker_dir: &Path) {
    let mut failures = 0_u32;
    for action in actions {
        let marker = match action.upgrade_marker {
            UpgradeMarkerAction::None => None,
            UpgradeMarkerAction::Publish => {
                Some(publish_service_upgrade_marker(marker_dir, &action.service))
            }
        };
        info!(
            service = %action.service,
            command = %action.command_path.display(),
            action = %action.action,
            "executing service action"
        );
        let result = Command::new(&action.command_path)
            .arg(&action.action)
            .status();
        match result {
            Ok(status) if status.success() => {
                info!(
                    service = %action.service,
                    action = %action.action,
                    "service action completed successfully"
                );
            }
            Ok(status) => {
                failures += 1;
                if let Some(marker) = &marker {
                    remove_failed_service_upgrade_marker(&action.service, marker);
                }
                if let Some(code) = status.code() {
                    error!(
                        service = %action.service,
                        action = %action.action,
                        exit_code = code,
                        "service action failed"
                    );
                } else {
                    error!(
                        service = %action.service,
                        action = %action.action,
                        "service action terminated by signal"
                    );
                }
            }
            Err(err) => {
                failures += 1;
                if let Some(marker) = &marker {
                    remove_failed_service_upgrade_marker(&action.service, marker);
                }
                error!(
                    service = %action.service,
                    action = %action.action,
                    error = %err,
                    "failed to run service action"
                );
            }
        }
    }
    if failures > 0 {
        warn!(
            failures,
            total = actions.len(),
            "some service actions failed"
        );
    }
}

fn delete_transient_service(instance_name: &str) -> anyhow::Result<()> {
    info!(instance_name, "deleting transient service");
    let payload = json!({ "name": instance_name }).to_string();
    let status = Command::new("ubus")
        .args(["call", "service", "delete", &payload])
        .status()
        .with_context(|| format!("failed to delete transient service '{instance_name}'"))?;

    if status.success() {
        Ok(())
    } else if let Some(code) = status.code() {
        anyhow::bail!(
            "ubus delete for transient service '{}' failed with exit code {}",
            instance_name,
            code
        )
    } else {
        anyhow::bail!(
            "ubus delete for transient service '{}' terminated by signal",
            instance_name
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::time::Duration;

    use bmc_nix::profile::lock_profile;
    use bmc_nix::service_orchestrator::{PlannedAction, ServiceConfig, UpgradeMarkerAction};
    use clap::Parser as _;
    use tempfile::tempdir;
    use tokio::sync::Mutex;
    use tokio::time::timeout;

    use super::{Args, execute_action_plan, run, verify_current_generation};

    #[test]
    fn upgrade_marker_is_visible_to_the_action() {
        let dir = tempdir().expect("BUG: should create temp dir");
        let marker_dir = dir.path().join("bmc-service-upgraded");
        let marker = marker_dir.join("bmc-compositor");
        let action = PlannedAction {
            priority: 0,
            service: "bmc-compositor".to_owned(),
            action: marker.to_string_lossy().into_owned(),
            command_path: PathBuf::from("cat"),
            upgrade_marker: UpgradeMarkerAction::Publish,
        };

        execute_action_plan(&[action], &marker_dir);

        assert!(
            marker.exists(),
            "the marker must exist before the restart action runs"
        );
    }

    #[test]
    fn failed_upgrade_action_removes_its_marker() {
        let dir = tempdir().expect("BUG: should create temp dir");
        let marker_dir = dir.path().join("bmc-service-upgraded");
        let marker = marker_dir.join("bmc-compositor");
        let action = PlannedAction {
            priority: 0,
            service: "bmc-compositor".to_owned(),
            action: "reload".to_owned(),
            command_path: PathBuf::from("false"),
            upgrade_marker: UpgradeMarkerAction::Publish,
        };

        execute_action_plan(&[action], &marker_dir);

        assert!(
            !marker.exists(),
            "a failed restart must not leave a marker for a later unrelated restart"
        );
    }

    #[test]
    fn parses_required_arguments_without_activation_pid() {
        let args = Args::try_parse_from([
            "bmc-nix-service-orchestrator",
            "--old-generation",
            "/nix/store/old",
            "--new-generation",
            "/nix/store/new",
            "--current-link",
            "/nix/var/nix/gcroots/profiles/bmc/current",
            "--instance-name",
            "bmc-nix-service-orchestrator",
            "--timeout-seconds",
            "30",
        ])
        .expect("BUG: argument parsing should succeed");

        assert_eq!(args.instance_name, "bmc-nix-service-orchestrator");
    }

    #[test]
    fn parses_empty_old_generation_as_first_activation_sentinel() {
        let args = Args::try_parse_from([
            "bmc-nix-service-orchestrator",
            "--old-generation=",
            "--new-generation=/nix/store/new",
            "--current-link=/nix/var/nix/gcroots/profiles/bmc/current",
            "--instance-name=bmc-nix-service-orchestrator",
            "--timeout-seconds=30",
        ])
        .expect("BUG: empty --old-generation must parse as a first-activation sentinel");

        assert!(args.old_generation.as_os_str().is_empty());
    }

    #[test]
    fn launcher_command_block_matches_orchestrator_cli_shape() {
        let launcher = fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../nix/pkgs/core/default.nix"
        ))
        .expect("BUG: should read launcher definition");

        let command_args = extract_launcher_command_args(&launcher);

        assert_eq!(
            command_args,
            [
                "$executable",
                "--old-generation=$PROFILE_OLD_GENERATION",
                "--new-generation=$PROFILE_NEW_GENERATION",
                "--current-link=$current_link",
                "--instance-name=<any>",
                "--timeout-seconds=300",
            ],
            "launcher should keep the expected orchestrator CLI shape"
        );
        assert!(
            !command_args
                .iter()
                .any(|arg| arg.starts_with("--activation-pid")),
            "launcher should stop passing the removed activation pid flag"
        );
    }

    #[test]
    fn library_service_config_defaults_are_available() {
        let config = ServiceConfig::default();

        assert_eq!(config.init, vec!["boot", "start"]);
        assert_eq!(config.removed, vec!["stop", "disable"]);
        assert_eq!(config.upgrade, vec!["disable", "reload"]);
        assert_eq!(config.always, vec!["enable"]);
    }

    #[test]
    fn verify_current_generation_accepts_matching_current_link() {
        let tmp = tempdir().expect("BUG: should create tempdir");
        let profile_dir = tmp.path().join("profile");
        let new_generation = profile_dir.join("2-link");
        let current_link = profile_dir.join("current");

        std::fs::create_dir_all(&new_generation).expect("BUG: should create generation");
        create_relative_symlink(Path::new("2-link"), &current_link);

        verify_current_generation(&current_link, &new_generation)
            .expect("BUG: matching current link should be accepted");
    }

    #[test]
    fn verify_current_generation_rejects_non_matching_current_link() {
        let tmp = tempdir().expect("BUG: should create tempdir");
        let profile_dir = tmp.path().join("profile");
        let old_generation = profile_dir.join("1-link");
        let new_generation = profile_dir.join("2-link");
        let current_link = profile_dir.join("current");

        std::fs::create_dir_all(&old_generation).expect("BUG: should create old generation");
        std::fs::create_dir_all(&new_generation).expect("BUG: should create new generation");
        create_relative_symlink(Path::new("1-link"), &current_link);

        let err = verify_current_generation(&current_link, &new_generation)
            .expect_err("BUG: mismatching current link should be rejected");

        assert!(
            err.to_string().contains("does not point to"),
            "error should explain the mismatching current link: {err:#}"
        );
    }

    #[tokio::test]
    async fn run_waits_for_profile_lock_without_activation_pid() {
        let _env_guard = test_env_lock().lock().await;
        let tmp = tempdir().expect("BUG: should create tempdir");
        let roots_dir = tmp.path();
        let profile_dir = roots_dir.join("profile");
        let old_generation = roots_dir.join("old");
        let new_generation = roots_dir.join("new");
        let current_link = profile_dir.join("current");

        create_generation_root(&old_generation);
        create_generation_root(&new_generation);
        create_absolute_symlink(&new_generation, &current_link);

        let _path_guard = prepend_path(&write_fake_ubus(roots_dir));

        let held_lock = lock_profile(&profile_dir)
            .await
            .expect("BUG: initial lock should succeed");
        let release_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(held_lock);
        });

        let args = Args {
            old_generation,
            new_generation,
            current_link,
            instance_name: "bmc-nix-service-orchestrator".to_owned(),
            timeout_seconds: 1,
        };

        timeout(Duration::from_secs(2), run(args))
            .await
            .expect("BUG: orchestrator should finish after lock release")
            .expect("BUG: run should succeed when current already points to new generation");

        release_task
            .await
            .expect("BUG: lock release task should not panic");
    }

    #[tokio::test]
    async fn run_fails_when_current_does_not_point_to_new_generation_after_lock() {
        let _env_guard = test_env_lock().lock().await;
        let tmp = tempdir().expect("BUG: should create tempdir");
        let roots_dir = tmp.path();
        let profile_dir = roots_dir.join("profile");
        let old_generation = roots_dir.join("old");
        let new_generation = roots_dir.join("new");
        let current_link = profile_dir.join("current");

        create_generation_root(&old_generation);
        create_generation_root(&new_generation);
        create_absolute_symlink(&old_generation, &current_link);

        let _path_guard = prepend_path(&write_fake_ubus(roots_dir));

        let held_lock = lock_profile(&profile_dir)
            .await
            .expect("BUG: initial lock should succeed");
        let release_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(held_lock);
        });

        let args = Args {
            old_generation,
            new_generation,
            current_link,
            instance_name: "bmc-nix-service-orchestrator".to_owned(),
            timeout_seconds: 1,
        };

        let err = timeout(Duration::from_secs(2), run(args))
            .await
            .expect("BUG: orchestrator should return before timeout")
            .expect_err("BUG: run should fail when current still points elsewhere");

        assert!(
            err.to_string().contains("does not point to"),
            "error should explain the mismatching current link: {err:#}"
        );

        release_task
            .await
            .expect("BUG: lock release task should not panic");
    }

    fn create_relative_symlink(target: &Path, link: &Path) {
        std::fs::create_dir_all(
            link.parent()
                .expect("BUG: symlink path should always have a parent"),
        )
        .expect("BUG: should create symlink parent directory");
        symlink(target, link).expect("BUG: should create symlink");
    }

    fn create_absolute_symlink(target: &Path, link: &Path) {
        std::fs::create_dir_all(
            link.parent()
                .expect("BUG: symlink path should always have a parent"),
        )
        .expect("BUG: should create symlink parent directory");
        symlink(target, link).expect("BUG: should create symlink");
    }

    fn create_generation_root(root: &Path) {
        fs::create_dir_all(root).expect("BUG: should create generation root");
    }

    fn write_fake_ubus(root: &Path) -> std::path::PathBuf {
        let bin_dir = root.join("fake-bin");
        fs::create_dir_all(&bin_dir).expect("BUG: should create fake bin dir");

        let ubus_path = bin_dir.join("ubus");
        fs::write(&ubus_path, b"#!/bin/sh\nexit 0\n").expect("BUG: should write fake ubus");
        let mut permissions = fs::metadata(&ubus_path)
            .expect("BUG: should stat fake ubus")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&ubus_path, permissions)
            .expect("BUG: should make fake ubus executable");

        bin_dir
    }

    fn prepend_path(prefix: &Path) -> PathGuard {
        let original = std::env::var_os("PATH");
        let mut new_path = OsString::from(prefix.as_os_str());
        if let Some(original_path) = original.as_ref() {
            new_path.push(":");
            new_path.push(original_path);
        }
        // SAFETY: these tests run the orchestrator in-process and restore PATH
        // when the guard drops. The helper is only used in isolated targeted tests.
        unsafe {
            std::env::set_var("PATH", &new_path);
        }
        PathGuard { original }
    }

    struct PathGuard {
        original: Option<OsString>,
    }

    impl Drop for PathGuard {
        fn drop(&mut self) {
            if let Some(original) = self.original.take() {
                // SAFETY: see `prepend_path`; the guard restores the original PATH.
                unsafe {
                    std::env::set_var("PATH", original);
                }
            } else {
                // SAFETY: see `prepend_path`; the guard removes PATH only if it was absent.
                unsafe {
                    std::env::remove_var("PATH");
                }
            }
        }
    }

    fn extract_launcher_command_args(launcher: &str) -> Vec<String> {
        let mut args = Vec::new();
        let mut in_start_service_block = false;
        let mut in_command_block = false;

        for line in launcher.lines() {
            if !in_start_service_block {
                if line.contains("start-service-orchestrator = armv7Pkgs.writeTextFile {") {
                    in_start_service_block = true;
                }
                continue;
            }

            if !in_command_block {
                if line.contains("\\\"command\\\": [") {
                    in_command_block = true;
                }
                continue;
            }

            if line.trim() == "]," || line.trim() == "\\\"]," {
                break;
            }

            let mut parts = line.split("\\\"");
            let _prefix = parts.next();
            while let Some(value) = parts.next() {
                args.push(value.to_string());
                let _separator = parts.next();
            }
        }

        let instance_index = args
            .iter()
            .position(|arg| arg.starts_with("--instance-name="))
            .expect("BUG: launcher should keep the instance-name flag");
        args[instance_index] = "--instance-name=<any>".to_owned();

        args
    }

    fn test_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn registration_roots(tmp: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let rcd = tmp.join("rc.d");
        let init_d = tmp.join("init.d");
        fs::create_dir_all(&rcd).expect("BUG: should create rc.d");
        fs::create_dir_all(&init_d).expect("BUG: should create init.d");
        (rcd, init_d)
    }

    #[test]
    fn registers_valid_start_link_at_any_priority() {
        let tmp = tempdir().expect("BUG: should create tempdir");
        let (rcd, init_d) = registration_roots(tmp.path());
        fs::write(init_d.join("svc"), "#!/bin/sh\n").expect("BUG: should write script");
        // Production links are relative (../init.d/<name>); the priority
        // deliberately differs from anything a generation would ship.
        symlink(Path::new("../init.d/svc"), rcd.join("S42svc")).expect("BUG: should create link");

        assert_eq!(
            super::collect_live_registrations(&rcd, &init_d),
            BTreeSet::from(["svc".to_owned()]),
            "any-priority S link resolving to the live script counts: rc \
             started the service through it regardless of the slot number"
        );
    }

    #[test]
    fn kill_only_link_is_not_a_start_registration() {
        let tmp = tempdir().expect("BUG: should create tempdir");
        let (rcd, init_d) = registration_roots(tmp.path());
        fs::write(init_d.join("svc"), "#!/bin/sh\n").expect("BUG: should write script");
        symlink(Path::new("../init.d/svc"), rcd.join("K80svc")).expect("BUG: should create link");

        assert!(
            super::collect_live_registrations(&rcd, &init_d).is_empty(),
            "parse_rcd_link_name accepts K links too, so the collector must \
             filter on the S prefix itself"
        );
    }

    #[test]
    fn dangling_link_is_not_a_registration() {
        let tmp = tempdir().expect("BUG: should create tempdir");
        let (rcd, init_d) = registration_roots(tmp.path());
        symlink(Path::new("../init.d/svc"), rcd.join("S95svc")).expect("BUG: should create link");

        assert!(
            super::collect_live_registrations(&rcd, &init_d).is_empty(),
            "rc cannot start a service through a dangling link"
        );
    }

    #[test]
    fn link_to_another_script_is_not_a_registration() {
        let tmp = tempdir().expect("BUG: should create tempdir");
        let (rcd, init_d) = registration_roots(tmp.path());
        fs::write(init_d.join("svc"), "#!/bin/sh\n").expect("BUG: should write script");
        fs::write(init_d.join("other"), "#!/bin/sh\n").expect("BUG: should write script");
        symlink(Path::new("../init.d/other"), rcd.join("S95svc")).expect("BUG: should create link");

        assert!(
            super::collect_live_registrations(&rcd, &init_d).is_empty(),
            "an S link named for svc but resolving to another script would \
             have started the wrong service"
        );
    }

    #[test]
    fn one_valid_candidate_among_stale_links_registers() {
        let tmp = tempdir().expect("BUG: should create tempdir");
        let (rcd, init_d) = registration_roots(tmp.path());
        fs::write(init_d.join("svc"), "#!/bin/sh\n").expect("BUG: should write script");
        symlink(Path::new("../init.d/gone"), rcd.join("S10svc")).expect("BUG: should create link");
        symlink(Path::new("../init.d/svc"), rcd.join("S95svc")).expect("BUG: should create link");

        assert_eq!(
            super::collect_live_registrations(&rcd, &init_d),
            BTreeSet::from(["svc".to_owned()]),
            "one resolving link is enough; stale siblings do not cancel it"
        );
    }

    #[test]
    fn missing_rcd_directory_means_everything_unregistered() {
        let tmp = tempdir().expect("BUG: should create tempdir");
        let (_rcd, init_d) = registration_roots(tmp.path());

        assert!(
            super::collect_live_registrations(&tmp.path().join("no-such-dir"), &init_d).is_empty(),
            "a wiped rc.d directory is the factory-reset case, not an error"
        );
    }

    #[test]
    fn digit_leading_service_name_stays_unregistered() {
        let tmp = tempdir().expect("BUG: should create tempdir");
        let (rcd, init_d) = registration_roots(tmp.path());
        fs::write(init_d.join("9to5"), "#!/bin/sh\n").expect("BUG: should write script");
        symlink(Path::new("../init.d/9to5"), rcd.join("S159to5")).expect("BUG: should create link");

        // Pins the pre-existing parser behavior: the greedy digit scan
        // reads "S159to5" as priority 159 + name "to5", so the name never
        // resolves. Discovery mis-parses the generation link the same way,
        // so such a service is also never promoted — consistent, if wrong.
        assert!(
            super::collect_live_registrations(&rcd, &init_d).is_empty(),
            "digit-leading service names are unresolvable by the shared \
             rc.d parser (its digit scan is greedy)"
        );
    }
}
