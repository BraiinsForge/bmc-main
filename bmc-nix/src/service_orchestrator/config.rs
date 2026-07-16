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

/// Gate that decides when upgrade actions should run for a changed service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeIfStatus {
    #[default]
    Running,
    Stopped,
    Always,
}

/// Per-service configuration for init/upgrade/removal actions.
///
/// `always` runs for every service present in the new generation on every
/// activation, regardless of whether the service changed. This is how we
/// reconcile `/etc/rc.d/S*` symlinks on every boot via `enable` (and
/// symmetrically how service removal tears them down via `disable`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct ServiceConfig {
    pub init: Vec<String>,
    pub removed: Vec<String>,
    pub upgrade: Vec<String>,
    pub always: Vec<String>,
    pub reboot_required: bool,
    pub upgrade_if_status: UpgradeIfStatus,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            init: vec!["boot".into(), "start".into()],
            removed: vec!["stop".into(), "disable".into()],
            // `disable` wipes every /etc/rc.d/[SK]??<name> entry (including
            // stale ones from a changed START/STOP priority); `always`'s
            // `enable` runs afterwards and recreates the correct symlink
            // at the new priority. Upgrades happen only during post-boot
            // activation, so the brief symlink gap does not race with rcS.
            upgrade: vec!["disable".into(), "reload".into()],
            always: vec!["enable".into()],
            reboot_required: false,
            upgrade_if_status: UpgradeIfStatus::Running,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_upgrade_if_status_to_running() {
        let config = ServiceConfig::default();

        assert_eq!(config.init, vec!["boot", "start"]);
        assert_eq!(config.removed, vec!["stop", "disable"]);
        assert_eq!(config.upgrade, vec!["disable", "reload"]);
        assert_eq!(config.always, vec!["enable"]);
        assert!(!config.reboot_required);
        assert_eq!(config.upgrade_if_status, UpgradeIfStatus::Running);
    }

    #[test]
    fn parses_upgrade_if_status_from_snake_case() {
        let config: ServiceConfig = serde_json::from_str(
            r#"{
                "init": ["custom-init"],
                "removed": ["stop", "disable"],
                "upgrade": ["reload"],
                "always": ["enable", "custom-always"],
                "reboot_required": true,
                "upgrade_if_status": "always"
            }"#,
        )
        .expect("BUG: service config should parse");

        assert_eq!(config.init, vec!["custom-init"]);
        assert_eq!(config.removed, vec!["stop", "disable"]);
        assert_eq!(config.always, vec!["enable", "custom-always"]);
        assert_eq!(config.upgrade_if_status, UpgradeIfStatus::Always);
        assert!(config.reboot_required);
    }

    #[test]
    fn missing_always_field_defaults_to_enable() {
        let config: ServiceConfig =
            serde_json::from_str("{}").expect("BUG: empty object should parse using defaults");
        assert_eq!(config.always, vec!["enable"]);
    }
}
