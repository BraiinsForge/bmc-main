// Copyright (C) 2026  Braiins Systems s.r.o.

use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;

use bmc_led::data::LedEvent;
use bmc_nix_init::config::InitConfig;
use bmc_nix_init::init::{EncryptionType, InitError, InitPlatform, WifiScanItem};

pub struct MockPlatform {
    wifi_connected: bool,
}

impl MockPlatform {
    pub fn new(wifi_connected: bool) -> Self {
        Self { wifi_connected }
    }
}

impl InitPlatform for MockPlatform {
    async fn has_wifi_configuration(&self) -> Result<bool, InitError> {
        Ok(self.wifi_connected)
    }

    fn set_setup_pending(&self) -> Result<(), InitError> {
        tracing::info!("mock: set_setup_pending");
        Ok(())
    }

    async fn ip_address(&self) -> Option<IpAddr> {
        Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
    }

    async fn configure_wifi_ap(&self) -> Result<String, InitError> {
        tracing::info!("mock: configure_wifi_ap");
        Ok("BraiinsDeck-Setup".to_owned())
    }

    async fn enable_captive_portal(&self) -> Result<(), InitError> {
        tracing::info!("mock: enable_captive_portal");
        Ok(())
    }

    async fn disable_captive_portal(&self) -> Result<(), InitError> {
        tracing::info!("mock: disable_captive_portal");
        Ok(())
    }

    async fn scan_wifi(&self) -> Result<Vec<WifiScanItem>, InitError> {
        tracing::info!("mock: scan_wifi");
        Ok(vec![
            WifiScanItem::new("HomeNetwork".to_owned(), -45, EncryptionType::Wpa2),
            WifiScanItem::new("CoffeeShop".to_owned(), -70, EncryptionType::Wpa2),
            WifiScanItem::new("OpenGuest".to_owned(), -80, EncryptionType::None),
        ])
    }

    async fn save_and_connect(
        &self,
        ssid: String,
        _password: Option<String>,
        _encryption: EncryptionType,
    ) -> Result<(), InitError> {
        tracing::info!("mock: save_and_connect ssid={ssid}");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if !self.wifi_connected {
            return Err(InitError::wifi("mock: no wifi"));
        }
        Ok(())
    }

    async fn wifi_ap_ssid(&self) -> Option<String> {
        Some("BraiinsDeck-Setup".to_owned())
    }

    async fn wifi_sta_ssid(&self) -> Option<String> {
        Some("MockNetwork".to_owned())
    }

    fn is_store_ever_initialized(&self, config: &InitConfig) -> bool {
        let nix_store = std::path::Path::new("/nix/store");
        let backing_store = config.nix_data_dir.as_ref().map(|d| d.join("store"));

        let nix_exists = nix_store.exists();
        let backing_exists = backing_store.as_ref().is_some_and(|p| p.exists());

        nix_exists && backing_exists
    }

    fn set_init_marker(&self, _config: &InitConfig) -> Result<(), InitError> {
        tracing::info!("mock: skipping U-Boot init marker");
        Ok(())
    }

    async fn prepare_nix_store(&self, _config: &InitConfig, _wipe: bool) -> Result<(), InitError> {
        tracing::info!("mock: skipping nix store mount");
        Ok(())
    }

    async fn activate_generation(&self, _profile_dir: &Path) -> Result<(), InitError> {
        tracing::info!("mock: pretending to activate generation");
        Ok(())
    }

    fn led_event_sender(&self) -> Option<tokio::sync::mpsc::Sender<LedEvent>> {
        None // no LEDs on mock
    }

    async fn bos_upgrade(
        &self,
        _image_path: &std::path::Path,
        _keep_settings: bool,
    ) -> Result<(), InitError> {
        tracing::info!("mock: BOS sysupgrade (no-op)");
        Ok(())
    }

    fn platform(&self) -> bmc_platform::BosPlatform {
        bmc_platform::BosPlatform::Bmc1
    }
}
