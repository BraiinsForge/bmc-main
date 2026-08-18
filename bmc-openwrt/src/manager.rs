// Copyright (C) 2025  Braiins Systems s.r.o.
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

use crate::pwd::{PasswordHashType, SHADOW_FILE_MODE, SHADOW_PATH, ShadowFile};
use crate::session::OpenwrtSessionManager;
use crate::uboot_env::UbootEnvManager;
use crate::unix::call_command;
use crate::unix::system_reboot;
use crate::{ROOT_USERNAME, pwd, unix};
use anyhow::anyhow;
use bmc::BmcManager;
use bmc::bootloader_config::BootloaderConfig;
use bmc::manager::{
    SERVICE_NAME_ENV, UpgradeError, UpgradeMarker, consume_upgrade_marker,
    service_upgrade_marker_path,
};
use bmc_net::NetworkManager;
use bmc_net::openwrt::UciNetworkManager;
use bmc_net_drv::wifi::WifiDriver;
use bmc_platform::serial_number::BoardSerial;
use bmc_platform::{BmcInfo, BosPlatform, BosVersion};
use bmc_shared_time::time::Timezone;
use bmc_support::PasswordProtectedZip;
use std::io;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tracing::{debug, error, info, trace, warn};

#[derive(Debug)]
#[expect(
    clippy::struct_field_names,
    reason = "the *_manager fields name the subsystem they own; renaming them would be less clear"
)]
pub struct Manager {
    bmc_info: Arc<Option<BmcInfo>>,
    /// Factory-burned OTP serial, `None` when it is unavailable or invalid.
    board_serial: Option<BoardSerial>,
    platform_override: Option<BosPlatform>,
    pub session_manager: OpenwrtSessionManager,
    timezone_sender: tokio::sync::watch::Sender<Timezone>,
    network_manager: Arc<UciNetworkManager>,
    uboot_env_manager: UbootEnvManager,
    upgrade_in_progress: AtomicBool,
}

impl Manager {
    const SYSUPGRADE_BIN: &str = "/sbin/sysupgrade";
    const SYSUPGRADE_ARG_NO_SAVE: &str = "-n";
    const UPGRADE_RESULT_FILE_PATH: &str = "/etc/upgrade_result";
    const DEFAULT_INTERFACE: &str = "wlan0";
    const NETWORK_SECTION: &str = "wifi_sta";

    const UCI_SYSTEM_ZONENAME: &str = "system.@system[0].zonename";
    const UCI_SYSTEM_TIMEZONE: &str = "system.@system[0].timezone";

    #[must_use]
    pub async fn new(
        session_manager: OpenwrtSessionManager,
        timezone: Timezone,
        wifi_manager: Option<Arc<dyn WifiDriver>>,
        platform_override: Option<BosPlatform>,
        board_serial: Option<BoardSerial>,
    ) -> Self {
        let (timezone_sender, _) = tokio::sync::watch::channel(timezone);

        let bmc_info = match BmcInfo::load() {
            Ok(bmc_info) => Some(bmc_info),
            Err(err) => {
                error!(error = ?err, "Failed to load BMC info");
                None
            }
        };

        // Resolve WiFi interface name once from wifi_manager (reads sysfs net/ dir).
        // Falls back to DEFAULT_INTERFACE if the device isn't ready yet, and on a
        // board without a radio, where there is no driver to ask.
        let wifi_iface_name = match wifi_manager.as_ref() {
            Some(wifi) => wifi.wifi_device_name().await.unwrap_or_else(|err| {
                error!(?err, "failed to resolve WiFi interface name, using default");
                Self::DEFAULT_INTERFACE.to_owned()
            }),
            None => Self::DEFAULT_INTERFACE.to_owned(),
        };

        // `new()` resolves the platform eagerly, so a missing /etc/bos_platform
        // without --hardware-profile panics here; this is intended.
        let platform = resolve_platform(platform_override, bmc_info.as_ref());
        let product_name = platform.product().display_name().to_owned();

        let network_manager = Arc::new(
            UciNetworkManager::new(
                Self::NETWORK_SECTION,
                wifi_manager,
                wifi_iface_name,
                product_name,
            )
            .await,
        );

        Self {
            bmc_info: Arc::new(bmc_info),
            board_serial,
            platform_override,
            session_manager,
            timezone_sender,
            network_manager,
            uboot_env_manager: UbootEnvManager::new(),
            upgrade_in_progress: AtomicBool::new(false),
        }
    }

    #[must_use]
    pub fn board_serial(&self) -> Option<BoardSerial> {
        self.board_serial
    }

    async fn run_sysupgrade(
        keep_settings: bool,
        upgrade_image_path: &Path,
        progress: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<(), UpgradeError> {
        let mut sysupgrade = Command::new(Self::SYSUPGRADE_BIN);
        if !keep_settings {
            sysupgrade.arg(Self::SYSUPGRADE_ARG_NO_SAVE);
        }
        sysupgrade.arg(upgrade_image_path.as_os_str());
        sysupgrade.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut handle = sysupgrade
            .spawn()
            .map_err(|e| UpgradeError::Failed(format!("failed to spawn sysupgrade: {e}")))?;
        let stdout = handle
            .stdout
            .take()
            .expect("BUG: stdout was piped but is missing");
        let stderr = handle
            .stderr
            .take()
            .expect("BUG: stderr was piped but is missing");

        // Drain both pipes concurrently with wait(): if either drain lagged
        // behind, the 64 KB pipe buffer could fill and block sysupgrade.
        let stdout_drain = drain_lines(stdout, |line| trace!(line, "sysupgrade stdout"));
        let stderr_drain = drain_lines(stderr, |line| {
            debug!(line, "sysupgrade stderr");
            if let Some(progress) = &progress {
                _ = progress.send(line.to_owned());
            }
        });
        let (status, (), ()) = tokio::join!(handle.wait(), stdout_drain, stderr_drain);

        let status = status
            .inspect_err(|err| error!(error = %err, "Upgrade process wait failed"))
            .map_err(|e| UpgradeError::Failed(format!("upgrade process wait failed: {e}")))?;

        interpret_sysupgrade_exit(status.code())
    }

    async fn restart_system_service(&self) -> anyhow::Result<()> {
        call_command("uci", &["commit", "system"]).await?;
        call_command("/etc/init.d/system", &["restart"]).await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl BmcManager for Manager {
    type SessionManager = OpenwrtSessionManager;
    type Error = Error;

    async fn version(&self) -> Option<BosVersion> {
        self.bmc_info
            .as_ref()
            .as_ref()
            .map(|bmc_info| bmc_info.bos_version.clone())
    }

    fn platform(&self) -> BosPlatform {
        resolve_platform(self.platform_override, self.bmc_info.as_ref().as_ref())
    }

    fn network_manager(&self) -> &dyn NetworkManager {
        self.network_manager.as_ref()
    }

    async fn upgrade(
        &self,
        keep_settings: bool,
        upgrade_image_path: &Path,
        progress: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<(), UpgradeError> {
        info!(
            keep_settings = keep_settings,
            path = %upgrade_image_path.display(),
            "Starting system upgrade"
        );

        self.upgrade_in_progress.store(true, Ordering::SeqCst);
        let result = Self::run_sysupgrade(keep_settings, upgrade_image_path, progress).await;
        if result.is_err() {
            self.upgrade_in_progress.store(false, Ordering::SeqCst);
        }
        result
    }

    async fn consume_upgrade_marker(&self) -> UpgradeMarker {
        consume_upgrade_marker(Path::new(Self::UPGRADE_RESULT_FILE_PATH)).await
    }

    async fn consume_service_upgrade_marker(&self) -> UpgradeMarker {
        let Some(marker) = service_upgrade_marker_path() else {
            warn!("{SERVICE_NAME_ENV} is unset, so an in-place upgrade cannot be reported");
            return UpgradeMarker::Absent;
        };
        consume_upgrade_marker(&marker).await
    }

    fn session_manager(&self) -> Self::SessionManager {
        self.session_manager.clone()
    }

    async fn check_password(&self, password: Option<&str>) -> Result<bool, Self::Error> {
        let shadow_file = ShadowFile::from_file(SHADOW_PATH)?;
        let matches = shadow_file.check_credentials(ROOT_USERNAME, password);

        Ok(matches)
    }

    async fn set_password(&self, password: Option<String>) -> Result<(), Self::Error> {
        let mut shadow_file = ShadowFile::from_file(SHADOW_PATH)?;
        shadow_file.set_password(ROOT_USERNAME, password, PasswordHashType::Md5)?;

        bmc::utils::replace_file_with_mode(
            SHADOW_PATH,
            shadow_file.to_string().as_bytes(),
            Some(SHADOW_FILE_MODE),
        )
        .await?;

        info!(username = ROOT_USERNAME, "System password updated");

        Ok(())
    }

    fn timezone(&self) -> Timezone {
        self.timezone_sender.borrow().clone()
    }

    async fn set_timezone(&self, timezone: Timezone) -> anyhow::Result<()> {
        let zonename_cmd = format!("{}={}", Self::UCI_SYSTEM_ZONENAME, timezone.iana());
        call_command("uci", &["set", &zonename_cmd]).await?;

        let timezone_cmd: String = format!("{}={}", Self::UCI_SYSTEM_TIMEZONE, timezone.posix());
        call_command("uci", &["set", &timezone_cmd]).await?;

        self.restart_system_service().await?;

        let timezone_for_log = timezone.clone();
        self.timezone_sender.send_if_modified(|current| {
            if *current != timezone {
                *current = timezone;
                return true;
            }
            false
        });

        info!(timezone = %timezone_for_log, "System timezone updated");

        Ok(())
    }

    fn watch_timezone_updates(&self) -> tokio::sync::watch::Receiver<Timezone> {
        self.timezone_sender.subscribe()
    }

    async fn factory_reset(&self, hard: bool) -> Result<(), Self::Error> {
        let mut args = vec!["factory_reset"];
        if hard {
            args.push("--hard");
        }
        call_command("bos", &args).await?;
        Ok(())
    }

    async fn reboot(&self) -> anyhow::Result<()> {
        system_reboot().await.map_err(|e| anyhow!(e))
    }

    async fn handle_graceful_shutdown(&self) {
        unix::handle_graceful_shutdown(&self.upgrade_in_progress).await;
    }

    fn support_archive(&self) -> impl AsyncRead + Send + Unpin + 'static {
        unix::get_support_archive(&PasswordProtectedZip)
    }

    async fn sync_boot_environment(&self, config: &BootloaderConfig) -> Result<(), Self::Error> {
        self.uboot_env_manager
            .sync(config)
            .await
            .map_err(|e| Error::UbootEnv(e.to_string()))
    }
}

/// Resolve the platform from an explicit override or the loaded BMC info.
fn resolve_platform(
    platform_override: Option<BosPlatform>,
    bmc_info: Option<&BmcInfo>,
) -> BosPlatform {
    platform_override.unwrap_or_else(|| {
        bmc_info.map(|info| info.bmc_platform).expect(
            "BUG: bmc-openwrt requires /etc/bos_platform; \
             use --hardware-profile to override during development",
        )
    })
}

/// `-UBUS_STATUS_CONNECTION_FAILED` (-10) as a shell exit code. sysupgrade's
/// last command is `ubus call system sysupgrade`; procd accepts the upgrade
/// without answering the call, so the connection drops and sysupgrade exits
/// 246 on the success path.
const SYSUPGRADE_EXIT_UBUS_CONNECTION_FAILED: i32 = 246;

/// Map sysupgrade's exit status to the upgrade outcome.
fn interpret_sysupgrade_exit(exit_code: Option<i32>) -> Result<(), UpgradeError> {
    match exit_code {
        Some(0) => {
            info!("System upgrade completed successfully");
            Ok(())
        }
        Some(SYSUPGRADE_EXIT_UBUS_CONNECTION_FAILED) => {
            info!(
                exit_code = SYSUPGRADE_EXIT_UBUS_CONNECTION_FAILED,
                "System upgrade accepted; reboot follows"
            );
            Ok(())
        }
        // Error code "1" is returned on BCB when using incompatible image, unsigned image or wrong signature keys
        Some(1) => {
            error!(exit_code = 1, "Upgrade failed: invalid firmware image");
            Err(UpgradeError::InvalidImage)
        }
        // procd answers genuine rejections; they surface as other ubus
        // status exit codes (2, 8, 9, ...).
        Some(code) => {
            error!(exit_code = code, "Upgrade failed");
            Err(UpgradeError::Failed(format!(
                "upgrade failed with exit code {code}"
            )))
        }
        None => {
            error!("Upgrade process terminated without exit code");
            Err(UpgradeError::Failed(
                "upgrade process terminated without exit code".to_owned(),
            ))
        }
    }
}

/// Drain `reader` line-by-line, feeding each line to `on_line`.
///
/// Reads raw bytes and converts lossily: sysupgrade output is not
/// guaranteed to be UTF-8, and the drain must survive every line until
/// EOF — an early stop lets the pipe buffer fill and blocks sysupgrade
/// mid-flash. An I/O error on the pipe ends the drain; `wait()` still
/// decides the run's outcome.
async fn drain_lines(reader: impl tokio::io::AsyncRead + Unpin, mut on_line: impl FnMut(&str)) {
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) => break,
            Ok(_) => {
                if buf.last() == Some(&b'\n') {
                    buf.pop();
                    if buf.last() == Some(&b'\r') {
                        buf.pop();
                    }
                }
                on_line(&String::from_utf8_lossy(&buf));
            }
            Err(err) => {
                debug!(error = %err, "draining sysupgrade output failed");
                break;
            }
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    ShadowFile(#[from] pwd::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Unix(#[from] unix::Error),
    #[error("U-Boot environment error: {0}")]
    UbootEnv(String),
}

#[cfg(test)]
mod tests {
    use super::{UpgradeError, drain_lines, interpret_sysupgrade_exit};

    #[tokio::test]
    async fn drain_lines_survives_non_utf8_output() {
        let mut lines: Vec<String> = Vec::new();
        drain_lines(&b"ok\nbad \xff byte\nafter\n"[..], |line| {
            lines.push(line.to_owned());
        })
        .await;
        assert_eq!(
            lines,
            vec![
                "ok".to_owned(),
                "bad \u{FFFD} byte".to_owned(),
                "after".to_owned(),
            ],
            "a non-UTF-8 byte must not stop the drain"
        );
    }

    #[test]
    fn sysupgrade_exit_zero_is_success() {
        assert!(interpret_sysupgrade_exit(Some(0)).is_ok());
    }

    #[test]
    fn sysupgrade_exit_246_is_the_accepted_handoff_not_a_failure() {
        // procd accepts the upgrade without answering the ubus call, so a
        // successful flash exits with -UBUS_STATUS_CONNECTION_FAILED (246).
        // Reporting it as a failure showed "Upgrade failed" on every
        // successful upgrade.
        assert!(
            interpret_sysupgrade_exit(Some(246)).is_ok(),
            "exit 246 is the success path: the ubus call is never answered"
        );
    }

    #[test]
    fn sysupgrade_exit_one_is_an_invalid_image() {
        assert!(
            matches!(
                interpret_sysupgrade_exit(Some(1)),
                Err(UpgradeError::InvalidImage)
            ),
            "exit 1 is the image-check rejection and must stay a distinct error"
        );
    }

    #[test]
    fn sysupgrade_ubus_rejection_codes_are_failures() {
        // procd answers genuine rejections, so ubus exits with the real
        // status code (2, 8, 9, ...) — those must not ride the 246
        // acceptance path.
        for code in [2, 8, 9] {
            assert!(
                matches!(
                    interpret_sysupgrade_exit(Some(code)),
                    Err(UpgradeError::Failed(_))
                ),
                "exit {code} is a procd rejection, not an accepted handoff"
            );
        }
    }

    #[test]
    fn sysupgrade_death_by_signal_is_a_failure() {
        assert!(matches!(
            interpret_sysupgrade_exit(None),
            Err(UpgradeError::Failed(_))
        ));
    }
}
