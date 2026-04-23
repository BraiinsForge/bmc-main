// Copyright (C) 2025  Braiins Systems s.r.o.

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
            upgrade: vec!["reload".into()],
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
        assert_eq!(config.upgrade, vec!["reload"]);
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
