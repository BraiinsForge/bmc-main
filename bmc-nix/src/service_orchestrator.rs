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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub mod config;
pub mod discovery;

pub use config::{ServiceConfig, UpgradeIfStatus};
pub use discovery::{
    DiscoveredService, DiscoveryError, GenerationServices, ServiceChange, ServiceChangeKind,
    ServiceChanges, ServicePriorityMap, compare_generation_services, discover_generation,
    parse_rcd_link_name,
};

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

#[must_use]
pub fn build_action_plan(
    changes: &ServiceChanges,
    upgrade_statuses: &BTreeMap<String, ServiceStatus>,
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
            });
        }
    }

    for change in &changes.upgraded {
        let service = effective_service_config(change);
        let status = upgrade_statuses
            .get(&change.name)
            .copied()
            .unwrap_or(ServiceStatus::Unknown);
        if !should_run_upgrade(service.upgrade_if_status, status) {
            continue;
        }
        let command_path = active_root_command_path(&change.name);
        let priority = 100 + order_priority(new_start_order, &change.name).unwrap_or_default();
        for action in &service.upgrade {
            plan.push(PlannedAction {
                priority,
                service: change.name.clone(),
                action: action.clone(),
                command_path: command_path.clone(),
            });
        }
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
            });
        }
    }

    plan.sort_by(|left, right| left.priority.cmp(&right.priority));

    plan
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
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

        let actions = build_action_plan(&changes, &statuses, &old.stop_order, &new.start_order);

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
        let actions = build_action_plan(&changes, &statuses, &old.stop_order, &new.start_order);

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
}
