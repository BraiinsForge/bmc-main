// Copyright (C) 2026  Braiins Systems s.r.o.

use std::net::IpAddr;
use std::path::Path;

use bmc_led::data::LedEvent;
use bmc_led::led_driver::spawn_led_event_loop;
use bmc_nix::profile;
use bmc_nix_init::config::InitConfig;
use bmc_nix_init::init::{EncryptionType, InitError, InitPlatform, WifiScanItem};
use bmc_shared_ii_net_drv::wifi::utils::WifiCommand;
use bmc_shared_ii_net_drv::wifi::{OpenwrtWifiManager, WifiMode};
use bmc_shared_ii_net_drv::{NetworkInterface, get_primary_interface};
use tokio::sync::OnceCell;

pub struct OpenwrtPlatform {
    wifi_path: String,
    wifi_manager: OnceCell<OpenwrtWifiManager>,
    led_event_tx: std::sync::OnceLock<Option<tokio::sync::mpsc::Sender<LedEvent>>>,
}

impl OpenwrtPlatform {
    pub fn new(wifi_path: String) -> Self {
        Self {
            wifi_path,
            wifi_manager: OnceCell::new(),
            led_event_tx: std::sync::OnceLock::new(),
        }
    }

    /// Initialize the LED driver and event handler.
    ///
    /// Must be called from within a tokio runtime (spawns tasks).
    fn init_led() -> tokio::sync::mpsc::Sender<LedEvent> {
        use bmc_led::led_driver::LedDriverFactory as _;
        let led_driver =
            bmc_led::apa102_spi::platform_led_driver::PlatformLedDriver::new("/dev/spidev0.0");
        spawn_led_event_loop(led_driver.0.command_sender)
    }

    /// Get or lazily initialize the WiFi manager.
    ///
    /// `OpenwrtWifiManager::new` spawns a netlink connection task via
    /// `tokio::spawn`, so it must be called from within a tokio runtime.
    /// By deferring creation to first use we avoid requiring a runtime
    /// at `OpenwrtPlatform` construction time.
    async fn wifi_manager(&self) -> Result<&OpenwrtWifiManager, InitError> {
        self.wifi_manager
            .get_or_try_init(|| async {
                OpenwrtWifiManager::new(&self.wifi_path)
                    .map_err(|e| InitError::wifi(format!("failed to init WiFi manager: {e}")))
            })
            .await
    }
}

impl InitPlatform for OpenwrtPlatform {
    async fn has_wifi_configuration(&self) -> Result<bool, InitError> {
        let wifi = self.wifi_manager().await?;
        let statuses = wifi
            .status_all()
            .await
            .map_err(|e| InitError::wifi(format!("wifi status failed: {e}")))?;
        let has_sta = statuses.into_iter().any(|s| {
            s.enabled
                && s.configuration
                    .as_ref()
                    .is_some_and(|cfg| cfg.mode == WifiMode::Station)
        });
        Ok(has_sta)
    }

    fn is_store_ever_initialized(&self, config: &InitConfig) -> bool {
        let nix_store = std::path::Path::new("/nix/store");
        let backing_store = config.nix_data_dir.as_ref().map(|d| d.join("store"));

        let nix_exists = nix_store.exists();
        let backing_exists = backing_store.as_ref().is_some_and(|p| p.exists());

        if !nix_exists {
            tracing::info!("/nix/store missing — store not initialized");
        }
        if !backing_exists {
            tracing::info!("backing store missing — store not initialized");
        }

        nix_exists && backing_exists
    }

    fn set_init_marker(&self, config: &InitConfig) -> Result<(), InitError> {
        // Flush filesystem buffers before persisting the marker. If the marker
        // lands before store data is durable, a crash leaves the system with
        // `nix_init=1` and a partial /nix/store, which `nix-factory-reset`
        // would then preserve on the next boot.
        // SAFETY: sync(2) has no preconditions and cannot fail.
        unsafe {
            libc::sync();
        }

        let status = std::process::Command::new("fw_setenv")
            .arg(&config.uboot_sentinel_var)
            .arg("1")
            .status()
            .map_err(|e| InitError::config(format!("failed to run fw_setenv: {e}")))?;
        if !status.success() {
            return Err(InitError::config(format!(
                "fw_setenv {var} 1 failed with {status}",
                var = config.uboot_sentinel_var
            )));
        }
        Ok(())
    }

    fn set_setup_pending(&self) -> Result<(), InitError> {
        let status = std::process::Command::new("sh")
            .args([
                "-c",
                ". /lib/functions/bos-defaults.sh && set_setup_pending",
            ])
            .status()
            .map_err(|e| InitError::config(format!("failed to run set_setup_pending: {e}")))?;
        if !status.success() {
            return Err(InitError::config(format!(
                "set_setup_pending failed with {status}"
            )));
        }
        Ok(())
    }

    async fn ip_address(&self) -> Option<IpAddr> {
        // 1. Try default interface by name
        if let Some(ip) = NetworkInterface::get_by_name("wlan0").and_then(|n| n.ipv4_address()) {
            return Some(ip);
        }
        // 2. Try WiFi device name from driver
        if let Ok(name) = self.wifi_manager().await.ok()?.get_wifi_device_name().await
            && let Some(ip) = NetworkInterface::get_by_name(&name).and_then(|n| n.ipv4_address())
        {
            return Some(ip);
        }
        // 3. Any non-loopback interface with an IP
        get_primary_interface().and_then(|n| n.ipv4_address())
    }

    async fn configure_wifi_ap(&self) -> Result<String, InitError> {
        // Get WiFi MAC address
        let output = tokio::process::Command::new("sh")
            .args(["-c", ". /lib/functions/bos-defaults.sh && wifi_mac"])
            .output()
            .await
            .map_err(|e| InitError::wifi(format!("failed to get wifi_mac: {e}")))?;

        if !output.status.success() {
            return Err(InitError::wifi(format!(
                "wifi_mac failed with {}",
                output.status
            )));
        }

        let mac = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let mac_no_delim = mac.replace(':', "");
        // MAC addresses are ASCII hex — byte indexing is safe.
        #[expect(clippy::string_slice)]
        let mac_short_id = mac_no_delim
            .len()
            .checked_sub(3)
            .map_or("UNK".to_owned(), |start_idx| {
                mac_no_delim[start_idx..].to_uppercase()
            });
        let ssid = format!("Braiins Deck {mac_short_id}");
        tracing::info!("AP SSID: {ssid}");

        let wifi = self.wifi_manager().await?;
        wifi.reset_config()
            .await
            .map_err(|e| InitError::wifi(format!("reset_config failed: {e}")))?;
        wifi.configure_ap_mode(ssid.clone(), None, EncryptionType::None)
            .await
            .map_err(|e| InitError::wifi(format!("configure_ap_mode failed: {e}")))?;
        wifi.enable_radio(true)
            .await
            .map_err(|e| InitError::wifi(format!("enable_radio failed: {e}")))?;
        WifiCommand::restart()
            .await
            .map_err(|e| InitError::wifi(format!("wifi restart failed: {e}")))?;

        Ok(ssid)
    }

    async fn enable_captive_portal(&self) -> Result<(), InitError> {
        let status = tokio::process::Command::new("sh")
            .args([
                "-c",
                ". /lib/functions/bos-factory-default.sh && enable_captive_portal $FACTORY_DEFAULT_AP_IP_ADDR && /etc/init.d/dnsmasq restart",
            ])
            .status()
            .await
            .map_err(|e| InitError::wifi(format!("failed to enable captive portal: {e}")))?;

        if !status.success() {
            return Err(InitError::wifi(format!(
                "enable_captive_portal failed with {status}"
            )));
        }
        Ok(())
    }

    async fn disable_captive_portal(&self) -> Result<(), InitError> {
        let status = tokio::process::Command::new("sh")
            .args([
                "-c",
                ". /lib/functions/bos-factory-default.sh && disable_captive_portal && /etc/init.d/dnsmasq restart",
            ])
            .status()
            .await
            .map_err(|e| InitError::wifi(format!("failed to disable captive portal: {e}")))?;

        if !status.success() {
            return Err(InitError::wifi(format!(
                "disable_captive_portal failed with {status}"
            )));
        }
        Ok(())
    }

    async fn scan_wifi(&self) -> Result<Vec<WifiScanItem>, InitError> {
        let wifi = self.wifi_manager().await?;
        let items = wifi
            .scan()
            .await
            .map_err(|e| InitError::wifi(format!("wifi scan failed: {e}")))?;
        Ok(items)
    }

    async fn save_and_connect(
        &self,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> Result<(), InitError> {
        let wifi = self.wifi_manager().await?;
        wifi.save_and_connect(ssid, password, encryption)
            .await
            .map_err(|e| InitError::wifi(format!("save_and_connect failed: {e}")))?;
        Ok(())
    }

    async fn wifi_ap_ssid(&self) -> Option<String> {
        let wifi = self.wifi_manager().await.ok()?;
        wifi.get_ap_ssid().await
    }

    async fn wifi_sta_ssid(&self) -> Option<String> {
        let wifi = self.wifi_manager().await.ok()?;
        wifi.get_sta_ssid().await
    }

    fn led_event_sender(&self) -> Option<tokio::sync::mpsc::Sender<LedEvent>> {
        self.led_event_tx
            .get_or_init(|| Some(Self::init_led()))
            .clone()
    }

    async fn prepare_nix_store(&self, config: &InitConfig, wipe: bool) -> Result<(), InitError> {
        let Some(data_nix) = &config.nix_data_dir else {
            return Ok(());
        };
        let nix = Path::new("/nix");

        if data_nix.exists() && wipe {
            tracing::info!("wiping existing store at {}", data_nix.display());
            let _ = tokio::process::Command::new("umount")
                .arg("/nix")
                .output()
                .await;
            std::fs::remove_dir_all(data_nix).map_err(|e| {
                InitError::config(format!("failed to clean {}: {e}", data_nix.display()))
            })?;
        }

        std::fs::create_dir_all(data_nix).map_err(|e| {
            InitError::config(format!("failed to create {}: {e}", data_nix.display()))
        })?;
        std::fs::create_dir_all(nix)
            .map_err(|e| InitError::config(format!("failed to create /nix: {e}")))?;

        let already_mounted = tokio::process::Command::new("mountpoint")
            .arg("-q")
            .arg("/nix")
            .status()
            .await
            .is_ok_and(|s| s.success());

        if already_mounted {
            tracing::info!("/nix already mounted, skipping bind mount");
        } else {
            let status = tokio::process::Command::new("mount")
                .args([
                    "--bind",
                    &data_nix.to_string_lossy(),
                    &nix.to_string_lossy(),
                ])
                .status()
                .await
                .map_err(|e| InitError::config(format!("failed to run mount: {e}")))?;

            if !status.success() {
                return Err(InitError::config(format!(
                    "mount --bind {} /nix failed with {status}",
                    data_nix.display()
                )));
            }

            tracing::info!("bind-mounted {} to /nix", data_nix.display());
        }

        Ok(())
    }

    async fn bos_upgrade(
        &self,
        image_path: &std::path::Path,
        keep_settings: bool,
    ) -> Result<(), InitError> {
        tracing::info!("starting BOS sysupgrade with {}", image_path.display());

        let mut cmd = tokio::process::Command::new("/sbin/sysupgrade");
        if !keep_settings {
            cmd.arg("-n");
        }
        cmd.arg(image_path);

        let status = cmd
            .status()
            .await
            .map_err(|e| InitError::config(format!("failed to run sysupgrade: {e}")))?;

        match status.code() {
            Some(0) | None => {
                // Success or killed by signal — sysupgrade triggers reboot.
                // Wait, then report error if we're still alive.
                tracing::info!("sysupgrade completed, awaiting reboot");
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Err(InitError::config("sysupgrade did not reboot as expected"))
            }
            Some(code) => Err(InitError::config(format!(
                "sysupgrade failed with exit code {code}"
            ))),
        }
    }

    fn platform(&self) -> bmc_platform::BosPlatform {
        bmc_platform::BosPlatform::Bmc1
    }

    async fn activate_generation(&self, profile_dir: &Path) -> Result<(), InitError> {
        let lock = profile::lock_profile(profile_dir)
            .await
            .map_err(|e| InitError::activation(format!("lock failed: {e}")))?;

        let (number, path) = find_latest_generation(profile_dir)?;

        profile::activate_profile(profile_dir, number, &path, Some(&lock))
            .await
            .map_err(|e| InitError::activation(format!("activation failed: {e}")))?;

        Ok(())
    }
}

fn find_latest_generation(profile_dir: &Path) -> Result<(usize, std::path::PathBuf), InitError> {
    let latest = profile::max_generation(profile_dir)
        .map_err(|e| InitError::activation(format!("failed to scan generations: {e}")))?
        .ok_or_else(|| InitError::activation("no generations found in profile directory"))?;
    Ok((latest, profile_dir.join(format!("{latest}-link"))))
}
