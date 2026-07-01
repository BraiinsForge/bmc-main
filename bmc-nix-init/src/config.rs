// Copyright (C) 2026  Braiins Systems s.r.o.

use std::path::PathBuf;

/// Configuration for the Nix init process.
#[derive(Debug)]
pub struct InitConfig {
    /// Path to servers.json
    pub servers_config_path: PathBuf,
    /// Path to BOS version file (e.g. /etc/bos_version)
    pub bos_version_path: PathBuf,
    /// Profile directory (e.g. /nix/var/nix/gcroots/profiles/bmc)
    pub profile_dir: PathBuf,
    /// Sentinel file indicating activation completed this boot.
    /// Lives on tmpfs so it is cleared on reboot.
    pub activation_sentinel: PathBuf,
    /// Directory for downloading tarballs (e.g. /mnt/data)
    pub download_dir: PathBuf,
    /// Persistent storage for the Nix store (e.g. /mnt/data/nix).
    /// Bind-mounted to /nix before initialization.
    /// Set to `None` to skip the bind mount (e.g. in mock mode).
    pub nix_data_dir: Option<PathBuf>,
    /// U-Boot environment variable name for init marker
    pub uboot_sentinel_var: String,
    /// Path to the frontend static files directory for the WiFi setup server.
    pub www_path: PathBuf,
    /// Whether to preserve device settings during sysupgrade.
    /// Default `false` matches the current `-n` (no save) behavior.
    pub keep_settings: bool,
    /// Path to inhibit file. If this file exists, init exits immediately.
    pub inhibit_init_path: PathBuf,
}

impl Default for InitConfig {
    fn default() -> Self {
        Self {
            servers_config_path: PathBuf::from("/etc/nix-upgrade/servers.json"),
            bos_version_path: PathBuf::from("/etc/bos_version"),
            profile_dir: PathBuf::from("/nix/var/nix/gcroots/profiles/bmc"),
            activation_sentinel: PathBuf::from("/tmp/nix_activated"),
            download_dir: PathBuf::from("/mnt/data"),
            nix_data_dir: Some(PathBuf::from("/mnt/data/nix")),
            uboot_sentinel_var: "nix_init".into(),
            www_path: PathBuf::from("/www/bmc"),
            keep_settings: false,
            inhibit_init_path: PathBuf::from("/mnt/data/NIX_INHIBIT_INIT"),
        }
    }
}

impl InitConfig {
    /// Data root used for staged store extraction.
    ///
    /// `nix_data_dir` is the bind-mount source for `/nix`; the staging
    /// directory is its parent so promotion creates `<data-root>/nix`.
    #[must_use]
    pub fn nix_stage_dir(&self) -> PathBuf {
        self.nix_data_dir
            .as_ref()
            .and_then(|path| path.parent())
            .map_or_else(|| self.download_dir.clone(), std::path::Path::to_path_buf)
    }

    /// Read the BOS version string from `/etc/bos_version`.
    ///
    /// Returns the complete version (e.g. `2026-03-04-0-8436f26b-26.02`).
    /// Used both for factory tarball matching (exact match against the
    /// factory index) and for upgrade sequence resolution.
    pub fn read_bos_version(&self) -> Result<String, std::io::Error> {
        Ok(std::fs::read_to_string(&self.bos_version_path)?
            .trim()
            .to_owned())
    }
}
