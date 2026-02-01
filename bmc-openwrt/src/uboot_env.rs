// Copyright (C) 2025  Braiins Systems s.r.o.

//! U-Boot environment manager for syncing bootloader configuration.
//!
//! This module handles writing night mode and display settings to U-Boot
//! environment variables using the `fw_setenv` and `fw_printenv` commands.

use anyhow::Context;
use bmc::bootloader_config::BootloaderConfig;
use std::collections::HashMap;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::{debug, warn};

/// U-Boot environment variable names
mod vars {
    pub const NIGHT_FROM: &str = "ii_user_night_from";
    pub const NIGHT_TO: &str = "ii_user_night_to";
    pub const LED_DAY: &str = "ii_user_led";
    pub const LED_NIGHT: &str = "ii_user_led_night";
    pub const SCREEN_DAY: &str = "ii_user_screen";
    pub const SCREEN_NIGHT: &str = "ii_user_screen_night";

    pub const ALL: &[&str] = &[
        NIGHT_FROM,
        NIGHT_TO,
        LED_DAY,
        LED_NIGHT,
        SCREEN_DAY,
        SCREEN_NIGHT,
    ];
}

/// Manager for U-Boot environment variables.
#[derive(Debug, Default)]
pub struct UbootEnvManager;

impl UbootEnvManager {
    /// Create a new U-Boot environment manager.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Sync bootloader configuration to U-Boot environment.
    pub async fn sync(&self, config: &BootloaderConfig) -> anyhow::Result<()> {
        let current = self.read_current_values().await?;
        let desired = Self::config_to_env_values(config);

        let needs_update = desired.iter().any(|(name, value)| match value {
            Some(value) => current.get(*name) != Some(value),
            None => current.contains_key(*name),
        });

        if !needs_update {
            debug!("U-Boot environment unchanged, no writes needed");
            return Ok(());
        }

        self.write_env_vars(&desired).await?;
        debug!("U-Boot environment updated");
        Ok(())
    }

    /// Write environment variables using fw_setenv batch mode (--script).
    async fn write_env_vars(&self, vars: &[(&str, Option<String>)]) -> anyhow::Result<()> {
        const SCRIPT_PATH: &str = "/tmp/uboot_env_script";

        // Build script content: "name value" for sets, "name" alone for unsets
        let mut script_content = String::new();

        for (name, value) in vars {
            if let Some(value) = value {
                script_content.push_str(name);
                script_content.push(' ');
                script_content.push_str(value);
                script_content.push('\n');
            } else {
                script_content.push_str(name);
                script_content.push('\n');
            }
        }

        // Write script to temp file and execute
        let mut file = tokio::fs::File::create(SCRIPT_PATH)
            .await
            .context("failed to create temp file for U-Boot env script")?;
        file.write_all(script_content.as_bytes())
            .await
            .context("failed to write U-Boot env script to temp file")?;
        file.flush()
            .await
            .context("failed to flush U-Boot env script temp file")?;

        debug!(script = %script_content.trim(), "Writing U-Boot env vars via script");

        let output = Command::new("fw_setenv")
            .arg("--script")
            .arg(SCRIPT_PATH)
            .output()
            .await?;

        // Clean up temp file, ignore errors
        let _ = tokio::fs::remove_file(SCRIPT_PATH).await;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(stderr = %stderr, "fw_setenv --script failed");
            anyhow::bail!("fw_setenv --script failed: {}", stderr);
        }

        Ok(())
    }

    /// Read current U-Boot environment values for our variables.
    async fn read_current_values(&self) -> anyhow::Result<HashMap<String, String>> {
        let output = Command::new("fw_printenv").args(vars::ALL).output().await?;

        let mut values = HashMap::new();

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Some((name, value)) = line.split_once('=') {
                    values.insert(name.to_owned(), value.to_owned());
                }
            }
        }

        Ok(values)
    }

    /// Convert BootloaderConfig to environment variable name-value pairs.
    /// None values indicate the variable should be unset (reset to default).
    fn config_to_env_values(config: &BootloaderConfig) -> Vec<(&'static str, Option<String>)> {
        vec![
            (
                vars::NIGHT_FROM,
                config.night_from_utc_minutes.map(|v| v.to_string()),
            ),
            (
                vars::NIGHT_TO,
                config.night_to_utc_minutes.map(|v| v.to_string()),
            ),
            (
                vars::LED_DAY,
                Some(if config.led_day { "1" } else { "0" }.to_owned()),
            ),
            (
                vars::LED_NIGHT,
                config.led_night.map(|v| if v { "1" } else { "0" }.to_owned()),
            ),
            (vars::SCREEN_DAY, Some(config.screen_day.to_string())),
            (vars::SCREEN_NIGHT, config.screen_night.map(|v| v.to_string())),
        ]
    }
}
