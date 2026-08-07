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

pub mod config;
pub mod discovery;

pub use config::{ServiceConfig, UpgradeIfStatus};
pub use discovery::{
    DiscoveredService, DiscoveryError, GenerationServices, ServiceChange, ServiceChangeKind,
    ServiceChanges, ServicePriorityMap, compare_generation_services, discover_generation,
    parse_rcd_link_name,
};

/// Directory where an activation publishes one marker
/// per service whose `upgrade` actions it ran.
/// Lives on tmpfs so markers never outlive the boot that produced them:
/// a service killed by a reboot, not by activation, must find none waiting.
pub const UPGRADED_SERVICE_MARKER_DIR: &str = "/dev/shm/bmc-service-upgraded";

/// Environment variable naming the service a process runs as.
/// `mkOpenWrtDaemon` sets it from the name it gives the init script,
/// so a daemon never carries its own copy of that name.
pub const SERVICE_NAME_ENV: &str = "BMC_SERVICE_NAME";

/// Marker announcing that an activation upgraded `service` in place.
#[must_use]
pub fn upgraded_service_marker(service: &str) -> PathBuf {
    Path::new(UPGRADED_SERVICE_MARKER_DIR).join(service)
}

/// Publish the marker announcing an in-place service upgrade.
pub fn publish_upgraded_service_marker(marker: &Path) -> std::io::Result<()> {
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(marker, [])
}

/// Runtime status returned by `/etc/init.d/<service> status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ServiceStatus {
    Running,
    Stopped,
    Unknown,
}

/// State of activation as inferred from the `current` profile symlink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationCompletion {
    Pending,
    Succeeded,
}

/// Concrete service action ready for command execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAction {
    pub priority: u16,
    pub service: String,
    pub action: String,
    pub command_path: PathBuf,
    pub upgrade_marker: UpgradeMarkerAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeMarkerAction {
    None,
    Publish,
}

#[must_use]
pub fn should_run_upgrade(gate: UpgradeIfStatus, status: ServiceStatus) -> bool {
    match gate {
        UpgradeIfStatus::Always => true,
        UpgradeIfStatus::Running => status == ServiceStatus::Running,
        UpgradeIfStatus::Stopped => status == ServiceStatus::Stopped,
    }
}

#[must_use]
pub fn evaluate_activation_completion(
    current_target: Option<PathBuf>,
    new_generation: &Path,
) -> ActivationCompletion {
    match current_target {
        Some(target) if target == new_generation => ActivationCompletion::Succeeded,
        _ => ActivationCompletion::Pending,
    }
}

fn old_generation_command_path(change: &ServiceChange) -> PathBuf {
    change
        .old
        .as_ref()
        .expect("BUG: removed services must contain the old generation service")
        .init_path
        .clone()
}

fn active_root_command_path(service: &str) -> PathBuf {
    Path::new("/etc/init.d").join(service)
}

fn effective_service_config(change: &ServiceChange) -> ServiceConfig {
    match change.kind {
        ServiceChangeKind::Removed => change
            .old
            .as_ref()
            .expect("BUG: removed services must contain the old generation service")
            .config
            .clone(),
        ServiceChangeKind::New | ServiceChangeKind::Upgraded | ServiceChangeKind::Unchanged => {
            change
                .new
                .as_ref()
                .expect("BUG: changed service should contain the new generation service")
                .config
                .clone()
        }
    }
}

fn order_priority(order: &ServicePriorityMap, service: &str) -> Option<u16> {
    order.get(service).copied()
}

/// Config of a changed service whose `upgrade` actions this activation runs,
/// or `None` when the status gate rejects it or it declares none.
fn planned_upgrade_config(
    change: &ServiceChange,
    upgrade_statuses: &BTreeMap<String, ServiceStatus>,
) -> Option<ServiceConfig> {
    let config = effective_service_config(change);
    let status = upgrade_statuses
        .get(&change.name)
        .copied()
        .unwrap_or(ServiceStatus::Unknown);
    if !should_run_upgrade(config.upgrade_if_status, status) || config.upgrade.is_empty() {
        return None;
    }
    Some(config)
}

#[must_use]
pub fn build_action_plan(
    changes: &ServiceChanges,
    upgrade_statuses: &BTreeMap<String, ServiceStatus>,
    live_registered: &BTreeSet<String>,
    old_stop_order: &ServicePriorityMap,
    new_start_order: &ServicePriorityMap,
) -> Vec<PlannedAction> {
    let mut plan = Vec::new();

    for change in &changes.removed {
        let service = effective_service_config(change);
        let command_path = old_generation_command_path(change);
        let priority = order_priority(old_stop_order, &change.name).unwrap_or_default();
        for action in &service.removed {
            plan.push(PlannedAction {
                priority,
                service: change.name.clone(),
                action: action.clone(),
                command_path: command_path.clone(),
                upgrade_marker: UpgradeMarkerAction::None,
            });
        }
    }

    for change in &changes.upgraded {
        let Some(service) = planned_upgrade_config(change, upgrade_statuses) else {
            continue;
        };
        let command_path = active_root_command_path(&change.name);
        let priority = 100 + order_priority(new_start_order, &change.name).unwrap_or_default();
        let Some((final_action, preceding_actions)) = service.upgrade.split_last() else {
            continue;
        };
        for action in preceding_actions {
            plan.push(PlannedAction {
                priority,
                service: change.name.clone(),
                action: action.clone(),
                command_path: command_path.clone(),
                upgrade_marker: UpgradeMarkerAction::None,
            });
        }
        plan.push(PlannedAction {
            priority,
            service: change.name.clone(),
            action: final_action.clone(),
            command_path,
            upgrade_marker: UpgradeMarkerAction::Publish,
        });
    }

    for change in &changes.new {
        let service = effective_service_config(change);
        let command_path = active_root_command_path(&change.name);
        let priority = 100 + order_priority(new_start_order, &change.name).unwrap_or_default();
        for action in &service.init {
            plan.push(PlannedAction {
                priority,
                service: change.name.clone(),
                action: action.clone(),
                command_path: command_path.clone(),
                upgrade_marker: UpgradeMarkerAction::None,
            });
        }
    }

    // Live-state reconciliation: a service the new generation enables
    // (has a start link for) but whose live rc.d registration is missing
    // was never started by rc — e.g. a soft factory reset wiped the
    // overlay links. Run its init actions as if it were new. Emitted
    // after the upgrade actions and before `always` so the stable sort
    // keeps upgrade → init → always within a priority bucket.
    for change in changes.upgraded.iter().chain(&changes.unchanged) {
        if live_registered.contains(&change.name) {
            continue;
        }
        let Some(start_priority) = order_priority(new_start_order, &change.name) else {
            continue;
        };
        let service = effective_service_config(change);
        let command_path = active_root_command_path(&change.name);
        let priority = 100 + start_priority;
        for action in &service.init {
            plan.push(PlannedAction {
                priority,
                service: change.name.clone(),
                action: action.clone(),
                command_path: command_path.clone(),
                upgrade_marker: UpgradeMarkerAction::None,
            });
        }
    }

    // `always` actions (default `["enable"]`) run for every service present
    // in the new generation on every activation, regardless of change kind.
    // Emitted LAST in source order so the stable sort-by-priority puts them
    // after any upgrade/init actions within the same priority bucket. That
    // way `upgrade = ["disable", "reload"]` can wipe stale rc.d entries and
    // `always = ["enable"]` reinstalls the correct symlink afterwards.
    for change in changes
        .new
        .iter()
        .chain(&changes.upgraded)
        .chain(&changes.unchanged)
    {
        let service = effective_service_config(change);
        let command_path = active_root_command_path(&change.name);
        let priority = 100 + order_priority(new_start_order, &change.name).unwrap_or_default();
        for action in &service.always {
            plan.push(PlannedAction {
                priority,
                service: change.name.clone(),
                action: action.clone(),
                command_path: command_path.clone(),
                upgrade_marker: UpgradeMarkerAction::None,
            });
        }
    }

    plan.sort_by_key(|left| left.priority);

    plan
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use super::*;

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("BUG: should create parent directories");
        }
        std::fs::write(path, contents).expect("BUG: should write file");
    }

    fn action_summary(actions: &[PlannedAction]) -> Vec<(u16, String, String, PathBuf)> {
        actions
            .iter()
            .map(|action| {
                (
                    action.priority,
                    action.service.clone(),
                    action.action.clone(),
                    action.command_path.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn publishing_upgrade_marker_creates_the_shared_artifact() {
        let dir = tempfile::tempdir().expect("BUG: should create temp dir");
        let marker = dir.path().join("markers/bmc-compositor");

        publish_upgraded_service_marker(&marker).expect("BUG: should publish marker");

        assert_eq!(
            std::fs::read(marker).expect("BUG: should read marker"),
            b"",
            "service consumers and the mock must observe the exact artifact production publishes"
        );
    }

    #[test]
    fn skips_upgrade_actions_when_running_gate_does_not_match() {
        let decision = should_run_upgrade(UpgradeIfStatus::Running, ServiceStatus::Stopped);

        assert!(!decision);
    }

    #[test]
    fn runs_upgrade_actions_when_always_gate_is_used() {
        let decision = should_run_upgrade(UpgradeIfStatus::Always, ServiceStatus::Stopped);

        assert!(decision);
    }

    #[test]
    fn activation_completes_only_after_current_points_to_new_generation() {
        let new_generation = Path::new("/nix/store/new");

        assert_eq!(
            evaluate_activation_completion(None, new_generation),
            ActivationCompletion::Pending
        );
        assert_eq!(
            evaluate_activation_completion(Some(PathBuf::from("/nix/store/old")), new_generation),
            ActivationCompletion::Pending
        );
        assert_eq!(
            evaluate_activation_completion(Some(PathBuf::from("/nix/store/new")), new_generation),
            ActivationCompletion::Succeeded
        );
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "exhaustive fixture asserting the full interleaved action plan"
    )]
    fn builds_action_plan_with_removed_and_priority_interleaving_for_upgraded_and_new_services() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        write_file(&old_root.join("etc/init.d/removed"), "old-removed");
        write_file(&old_root.join("etc/init.d/gated"), "old-gated");
        write_file(
            &old_root.join("etc/init.d/late-upgrade"),
            "old-late-upgrade",
        );
        write_file(&old_root.join("etc/rc.d/K20removed"), "");
        write_file(&old_root.join("etc/rc.d/S10gated"), "");
        write_file(&old_root.join("etc/rc.d/S30late-upgrade"), "");

        write_file(
            &old_root.join("etc/init.d.conf/removed.json"),
            r#"{"removed":["stop","disable"]}"#,
        );

        write_file(&new_root.join("etc/init.d/gated"), "new-gated");
        write_file(&new_root.join("etc/init.d/early-new"), "new-early-new");
        write_file(
            &new_root.join("etc/init.d/late-upgrade"),
            "new-late-upgrade",
        );
        write_file(&new_root.join("etc/rc.d/S10gated"), "");
        write_file(&new_root.join("etc/rc.d/S20early-new"), "");
        write_file(&new_root.join("etc/rc.d/S30late-upgrade"), "");
        write_file(
            &new_root.join("etc/init.d.conf/gated.json"),
            r#"{"upgrade":["reload"],"upgrade_if_status":"running"}"#,
        );
        write_file(
            &new_root.join("etc/init.d.conf/late-upgrade.json"),
            r#"{"upgrade":["restart"],"upgrade_if_status":"always"}"#,
        );
        write_file(
            &new_root.join("etc/init.d.conf/early-new.json"),
            r#"{"init":["enable","start"]}"#,
        );

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        let statuses = BTreeMap::from([
            ("gated".to_owned(), ServiceStatus::Stopped),
            ("late-upgrade".to_owned(), ServiceStatus::Stopped),
        ]);

        let actions = build_action_plan(
            &changes,
            &statuses,
            &BTreeSet::from(["gated".to_owned(), "late-upgrade".to_owned()]),
            &old.stop_order,
            &new.start_order,
        );

        assert_eq!(
            action_summary(&actions),
            vec![
                (
                    20,
                    "removed".to_owned(),
                    "stop".to_owned(),
                    old_root.join("etc/init.d/removed"),
                ),
                (
                    20,
                    "removed".to_owned(),
                    "disable".to_owned(),
                    old_root.join("etc/init.d/removed"),
                ),
                (
                    110,
                    "gated".to_owned(),
                    "enable".to_owned(),
                    PathBuf::from("/etc/init.d/gated"),
                ),
                (
                    120,
                    "early-new".to_owned(),
                    "enable".to_owned(),
                    PathBuf::from("/etc/init.d/early-new"),
                ),
                (
                    120,
                    "early-new".to_owned(),
                    "start".to_owned(),
                    PathBuf::from("/etc/init.d/early-new"),
                ),
                (
                    120,
                    "early-new".to_owned(),
                    "enable".to_owned(),
                    PathBuf::from("/etc/init.d/early-new"),
                ),
                (
                    130,
                    "late-upgrade".to_owned(),
                    "restart".to_owned(),
                    PathBuf::from("/etc/init.d/late-upgrade"),
                ),
                (
                    130,
                    "late-upgrade".to_owned(),
                    "enable".to_owned(),
                    PathBuf::from("/etc/init.d/late-upgrade"),
                ),
            ]
        );
    }

    #[test]
    fn always_actions_run_for_unchanged_services() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        // Same content in both generations — `kept` will be classified as
        // Unchanged by the discovery layer.
        write_file(&old_root.join("etc/init.d/kept"), "kept-service");
        write_file(&new_root.join("etc/init.d/kept"), "kept-service");
        write_file(&old_root.join("etc/rc.d/S15kept"), "");
        write_file(&new_root.join("etc/rc.d/S15kept"), "");

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        assert_eq!(
            changes.unchanged.len(),
            1,
            "kept service should be classified as unchanged"
        );

        let actions = build_action_plan(
            &changes,
            &BTreeMap::new(),
            &BTreeSet::from(["kept".to_owned()]),
            &old.stop_order,
            &new.start_order,
        );

        assert_eq!(
            action_summary(&actions),
            vec![(
                115,
                "kept".to_owned(),
                "enable".to_owned(),
                PathBuf::from("/etc/init.d/kept"),
            )],
            "unchanged service should run default always=[\"enable\"] at 100+start_order"
        );
    }

    #[test]
    fn upgrade_with_priority_change_runs_disable_then_enable() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        // Same service name, content bytes differ (simulating a START
        // change), and the rc.d link moves from S95 to S90.
        write_file(&old_root.join("etc/init.d/moved"), "START=95");
        write_file(&old_root.join("etc/rc.d/S95moved"), "");
        write_file(&new_root.join("etc/init.d/moved"), "START=90");
        write_file(&new_root.join("etc/rc.d/S90moved"), "");

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        assert_eq!(
            changes.upgraded.len(),
            1,
            "moved service with changed content should classify as upgraded"
        );

        let statuses = BTreeMap::from([("moved".to_owned(), ServiceStatus::Running)]);
        let actions = build_action_plan(
            &changes,
            &statuses,
            &BTreeSet::from(["moved".to_owned()]),
            &old.stop_order,
            &new.start_order,
        );

        assert_eq!(
            action_summary(&actions),
            vec![
                (
                    190,
                    "moved".to_owned(),
                    "disable".to_owned(),
                    PathBuf::from("/etc/init.d/moved"),
                ),
                (
                    190,
                    "moved".to_owned(),
                    "reload".to_owned(),
                    PathBuf::from("/etc/init.d/moved"),
                ),
                (
                    190,
                    "moved".to_owned(),
                    "enable".to_owned(),
                    PathBuf::from("/etc/init.d/moved"),
                ),
            ],
            "upgrade must wipe every [SK]??<name> (including stale S95) and \
             always=[enable] must recreate the symlink at the new priority"
        );
    }

    #[test]
    fn only_final_upgrade_action_publishes_service_upgrade_marker() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        write_file(&old_root.join("etc/init.d/moved"), "old");
        write_file(&old_root.join("etc/rc.d/S90moved"), "");
        write_file(&new_root.join("etc/init.d/moved"), "new");
        write_file(&new_root.join("etc/rc.d/S90moved"), "");

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);
        let statuses = BTreeMap::from([("moved".to_owned(), ServiceStatus::Running)]);

        let actions = build_action_plan(
            &changes,
            &statuses,
            &BTreeSet::from(["moved".to_owned()]),
            &old.stop_order,
            &new.start_order,
        );

        assert_eq!(
            actions
                .iter()
                .map(|action| (action.action.as_str(), action.upgrade_marker))
                .collect::<Vec<_>>(),
            vec![
                ("disable", UpgradeMarkerAction::None),
                ("reload", UpgradeMarkerAction::Publish),
                ("enable", UpgradeMarkerAction::None),
            ]
        );
    }

    #[test]
    fn unchanged_service_runs_always_actions() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        write_file(&old_root.join("etc/init.d/kept"), "kept-service");
        write_file(&new_root.join("etc/init.d/kept"), "kept-service");
        write_file(&old_root.join("etc/rc.d/S15kept"), "");
        write_file(&new_root.join("etc/rc.d/S15kept"), "");

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        let actions = build_action_plan(
            &changes,
            &BTreeMap::new(),
            &BTreeSet::from(["kept".to_owned()]),
            &old.stop_order,
            &new.start_order,
        );

        assert!(
            !actions.is_empty(),
            "unchanged service should still run its always actions"
        );
    }

    #[test]
    fn upgrade_marker_is_planned_only_when_upgrade_actions_run() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        write_file(&old_root.join("etc/init.d/moved"), "START=95");
        write_file(&old_root.join("etc/rc.d/S95moved"), "");
        write_file(&new_root.join("etc/init.d/moved"), "START=90");
        write_file(&new_root.join("etc/rc.d/S90moved"), "");

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        let running = BTreeMap::from([("moved".to_owned(), ServiceStatus::Running)]);
        let stopped = BTreeMap::from([("moved".to_owned(), ServiceStatus::Stopped)]);

        let registered = BTreeSet::from(["moved".to_owned()]);
        let running_actions = build_action_plan(
            &changes,
            &running,
            &registered,
            &old.stop_order,
            &new.start_order,
        );
        let stopped_actions = build_action_plan(
            &changes,
            &stopped,
            &registered,
            &old.stop_order,
            &new.start_order,
        );
        assert!(
            running_actions
                .iter()
                .any(|action| action.upgrade_marker == UpgradeMarkerAction::Publish),
            "a running upgraded service must receive a restart marker"
        );
        assert!(
            stopped_actions
                .iter()
                .all(|action| action.upgrade_marker == UpgradeMarkerAction::None),
            "the default running gate skips a stopped service, so nothing restarts it"
        );
    }

    #[test]
    fn service_declaring_no_upgrade_actions_has_no_upgrade_marker() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        write_file(&old_root.join("etc/init.d/inert"), "old-inert");
        write_file(&old_root.join("etc/rc.d/S15inert"), "");
        write_file(&new_root.join("etc/init.d/inert"), "new-inert");
        write_file(&new_root.join("etc/rc.d/S15inert"), "");
        write_file(
            &new_root.join("etc/init.d.conf/inert.json"),
            r#"{"upgrade":[]}"#,
        );

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        let statuses = BTreeMap::from([("inert".to_owned(), ServiceStatus::Running)]);
        let actions = build_action_plan(
            &changes,
            &statuses,
            &BTreeSet::from(["inert".to_owned()]),
            &old.stop_order,
            &new.start_order,
        );

        assert!(
            actions
                .iter()
                .all(|action| action.upgrade_marker == UpgradeMarkerAction::None),
            "a changed service with no upgrade actions must not publish an upgrade marker"
        );
    }

    #[test]
    fn always_actions_skipped_for_removed_services() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        write_file(&old_root.join("etc/init.d/gone"), "gone-service");
        write_file(&old_root.join("etc/rc.d/S15gone"), "");

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        let actions = build_action_plan(
            &changes,
            &BTreeMap::new(),
            &BTreeSet::new(),
            &old.stop_order,
            &new.start_order,
        );

        assert!(
            !actions.iter().any(|a| a.action == "enable"),
            "removed services must not run `always` actions like enable"
        );
    }

    #[test]
    fn removed_services_use_old_generation_config() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        write_file(&old_root.join("etc/init.d/removed"), "old-removed");
        write_file(&old_root.join("etc/rc.d/K20removed"), "");
        write_file(
            &old_root.join("etc/init.d.conf/removed.json"),
            r#"{"removed":["stop","disable"]}"#,
        );

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        let actions = build_action_plan(
            &changes,
            &BTreeMap::new(),
            &BTreeSet::new(),
            &old.stop_order,
            &new.start_order,
        );

        assert_eq!(
            action_summary(&actions),
            vec![
                (
                    20,
                    "removed".to_owned(),
                    "stop".to_owned(),
                    old_root.join("etc/init.d/removed"),
                ),
                (
                    20,
                    "removed".to_owned(),
                    "disable".to_owned(),
                    old_root.join("etc/init.d/removed"),
                ),
            ]
        );
    }

    #[test]
    fn promotes_unregistered_unchanged_service_with_init_actions() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        write_file(&old_root.join("etc/init.d/kept"), "kept-service");
        write_file(&new_root.join("etc/init.d/kept"), "kept-service");
        write_file(&old_root.join("etc/rc.d/S15kept"), "");
        write_file(&new_root.join("etc/rc.d/S15kept"), "");

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        let actions = build_action_plan(
            &changes,
            &BTreeMap::new(),
            &BTreeSet::new(),
            &old.stop_order,
            &new.start_order,
        );

        assert_eq!(
            action_summary(&actions),
            vec![
                (
                    115,
                    "kept".to_owned(),
                    "boot".to_owned(),
                    PathBuf::from("/etc/init.d/kept"),
                ),
                (
                    115,
                    "kept".to_owned(),
                    "start".to_owned(),
                    PathBuf::from("/etc/init.d/kept"),
                ),
                (
                    115,
                    "kept".to_owned(),
                    "enable".to_owned(),
                    PathBuf::from("/etc/init.d/kept"),
                ),
            ],
            "an unchanged service without a live start registration must run \
             its init actions (a factory reset wiped the links rc would have \
             started it from) and still get the always=[enable] reconcile"
        );
    }

    #[test]
    fn does_not_promote_registered_unchanged_service() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        write_file(&old_root.join("etc/init.d/kept"), "kept-service");
        write_file(&new_root.join("etc/init.d/kept"), "kept-service");
        write_file(&old_root.join("etc/rc.d/S15kept"), "");
        write_file(&new_root.join("etc/rc.d/S15kept"), "");

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        let actions = build_action_plan(
            &changes,
            &BTreeMap::new(),
            &BTreeSet::from(["kept".to_owned()]),
            &old.stop_order,
            &new.start_order,
        );

        assert_eq!(
            action_summary(&actions),
            vec![(
                115,
                "kept".to_owned(),
                "enable".to_owned(),
                PathBuf::from("/etc/init.d/kept"),
            )],
            "a registered service is rc's to start: a healthy boot must stay \
             a no-op apart from the always=[enable] reconcile"
        );
    }

    #[test]
    fn does_not_promote_service_without_generation_start_link() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        // No etc/rc.d S link in either generation: the generation itself
        // says "not enabled", so a missing live link is the desired state.
        write_file(&old_root.join("etc/init.d/optout"), "optout-service");
        write_file(&new_root.join("etc/init.d/optout"), "optout-service");

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        let actions = build_action_plan(
            &changes,
            &BTreeMap::new(),
            &BTreeSet::new(),
            &old.stop_order,
            &new.start_order,
        );

        assert!(
            !actions
                .iter()
                .any(|a| a.action == "boot" || a.action == "start"),
            "a service the generation ships no S link for must never be \
             promoted, regardless of its live registration: {:?}",
            action_summary(&actions)
        );
    }

    #[test]
    fn promoted_upgraded_service_with_always_gate_orders_upgrade_init_always() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        write_file(&old_root.join("etc/init.d/mig"), "old-mig");
        write_file(&new_root.join("etc/init.d/mig"), "new-mig");
        write_file(&old_root.join("etc/rc.d/S30mig"), "");
        write_file(&new_root.join("etc/rc.d/S30mig"), "");
        write_file(
            &new_root.join("etc/init.d.conf/mig.json"),
            r#"{"upgrade":["disable","reload"],"upgrade_if_status":"always"}"#,
        );

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        let statuses = BTreeMap::from([("mig".to_owned(), ServiceStatus::Stopped)]);
        let actions = build_action_plan(
            &changes,
            &statuses,
            &BTreeSet::new(),
            &old.stop_order,
            &new.start_order,
        );

        let sequence: Vec<&str> = actions.iter().map(|a| a.action.as_str()).collect();
        assert_eq!(
            sequence,
            vec!["disable", "reload", "boot", "start", "enable"],
            "an upgrade staged across a factory reset must keep its gated \
             upgrade (migration) actions, then the promoted init actions, \
             then the always reconcile"
        );
    }

    #[test]
    fn promoted_upgraded_service_default_gate_runs_init_without_upgrade_actions() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        write_file(&old_root.join("etc/init.d/gated"), "old-gated");
        write_file(&new_root.join("etc/init.d/gated"), "new-gated");
        write_file(&old_root.join("etc/rc.d/S30gated"), "");
        write_file(&new_root.join("etc/rc.d/S30gated"), "");
        write_file(
            &new_root.join("etc/init.d.conf/gated.json"),
            r#"{"upgrade":["reload"]}"#,
        );

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        let statuses = BTreeMap::from([("gated".to_owned(), ServiceStatus::Stopped)]);
        let actions = build_action_plan(
            &changes,
            &statuses,
            &BTreeSet::new(),
            &old.stop_order,
            &new.start_order,
        );

        let sequence: Vec<&str> = actions.iter().map(|a| a.action.as_str()).collect();
        assert_eq!(
            sequence,
            vec!["boot", "start", "enable"],
            "the default running gate skips upgrade actions for a stopped \
             service, but the missing registration must still promote it"
        );
    }

    #[test]
    fn promoted_upgraded_service_with_stopped_gate_orders_upgrade_init_always() {
        let tempdir = tempfile::tempdir().expect("BUG: should create temp dir");
        let old_root = tempdir.path().join("old");
        let new_root = tempdir.path().join("new");

        write_file(&old_root.join("etc/init.d/mig"), "old-mig");
        write_file(&new_root.join("etc/init.d/mig"), "new-mig");
        write_file(&old_root.join("etc/rc.d/S30mig"), "");
        write_file(&new_root.join("etc/rc.d/S30mig"), "");
        write_file(
            &new_root.join("etc/init.d.conf/mig.json"),
            r#"{"upgrade":["reload"],"upgrade_if_status":"stopped"}"#,
        );

        let old = discover_generation(&old_root).expect("BUG: should discover old generation");
        let new = discover_generation(&new_root).expect("BUG: should discover new generation");
        let changes = compare_generation_services(&old, &new);

        let statuses = BTreeMap::from([("mig".to_owned(), ServiceStatus::Stopped)]);
        let actions = build_action_plan(
            &changes,
            &statuses,
            &BTreeSet::new(),
            &old.stop_order,
            &new.start_order,
        );

        let sequence: Vec<&str> = actions.iter().map(|a| a.action.as_str()).collect();
        assert_eq!(
            sequence,
            vec!["reload", "boot", "start", "enable"],
            "a stopped-gated upgrade action runs before the promoted init \
             actions within the same priority bucket"
        );
    }
}
