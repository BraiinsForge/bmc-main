// Copyright (C) 2026  Braiins Systems s.r.o.

use std::path::Path;
use std::sync::Arc;

use bmc_nix::store;
pub use bmc_shared_ii_net::wifi::{EncryptionType, WifiScanItem};

use bmc_led::data::LedEvent;

use crate::config::InitConfig;
use crate::state::{InitState, InitStateObserver};

/// Platform abstraction for hardware-specific operations.
///
/// The openwrt binary provides real implementations (WiFi via wlan0,
/// U-Boot env via fw_printenv/fw_setenv, bind mount, real activation).
/// The mock binary provides fake implementations that work in temp dirs.
///
/// This trait mirrors master's `BmcManager` but only includes what
/// the init binary needs. The library crate orchestrates these
/// primitives in `run_init()`.
pub trait InitPlatform: Send + Sync {
    // ── Store lifecycle ─────────────────────────────────────────

    /// Check if the store was ever initialized (persists across reboots).
    fn is_store_ever_initialized(&self, config: &InitConfig) -> bool;

    /// Set persistent init marker after successful first-time init.
    fn set_init_marker(&self, config: &InitConfig) -> Result<(), InitError>;

    /// Mount the promoted Nix store directory (e.g. bind mount
    /// /mnt/data/nix to /nix).
    fn mount_nix_store(
        &self,
        config: &InitConfig,
    ) -> impl std::future::Future<Output = Result<(), InitError>> + Send;

    /// Activate the latest profile generation in the given profile directory.
    fn activate_generation(
        &self,
        profile_dir: &Path,
    ) -> impl std::future::Future<Output = Result<(), InitError>> + Send;

    // ── Device state ────────────────────────────────────────────

    /// Set device state to SetupPending so `bmc` runs device setup
    /// (timezone, password, etc.) when it starts after init exits.
    fn set_setup_pending(&self) -> Result<(), InitError>;

    // ── Network ─────────────────────────────────────────────────

    /// Check if a STA WiFi network is configured (i.e. user previously
    /// set up a WiFi connection). When true, the system should wait for
    /// the connection to come up before falling back to AP mode.
    fn has_wifi_configuration(
        &self,
    ) -> impl std::future::Future<Output = Result<bool, InitError>> + Send;

    /// Get device IP address (for captive portal redirect).
    fn ip_address(&self) -> impl std::future::Future<Output = Option<std::net::IpAddr>> + Send;

    // ── AP mode lifecycle ───────────────────────────────────────

    /// Configure WiFi radio as AP. Steps:
    /// 1. Get MAC, generate SSID
    /// 2. reset_config (delete wireless, regenerate, poll)
    /// 3. configure_ap_mode (radio + iface via ubus)
    /// 4. enable_radio(true)
    ///
    /// Returns the AP SSID. Does NOT enable captive portal DNS.
    fn configure_wifi_ap(
        &self,
    ) -> impl std::future::Future<Output = Result<String, InitError>> + Send;

    /// Enable captive portal DNS redirect + dnsmasq restart.
    fn enable_captive_portal(
        &self,
    ) -> impl std::future::Future<Output = Result<(), InitError>> + Send;

    /// Disable captive portal DNS redirect + dnsmasq restart.
    fn disable_captive_portal(
        &self,
    ) -> impl std::future::Future<Output = Result<(), InitError>> + Send;

    // ── WiFi operations (for gRPC handlers) ─────────────────────

    /// Scan for WiFi networks. Filters: only APs, empty SSIDs,
    /// WPA3-only.
    fn scan_wifi(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<WifiScanItem>, InitError>> + Send;

    /// Configure STA mode and connect to a WiFi network.
    /// Saves UCI config, triggers wifi reload, polls for IP.
    fn save_and_connect(
        &self,
        ssid: String,
        password: Option<String>,
        encryption: EncryptionType,
    ) -> impl std::future::Future<Output = Result<(), InitError>> + Send;

    /// Read the current AP SSID from wireless config.
    /// Returns None if no AP interface is configured.
    fn wifi_ap_ssid(&self) -> impl std::future::Future<Output = Option<String>> + Send;

    /// Read the configured STA SSID from wireless config.
    /// Returns None if no STA interface is configured.
    fn wifi_sta_ssid(&self) -> impl std::future::Future<Output = Option<String>> + Send;

    /// Get LED event sender, if LED hardware is available.
    /// Returns None on mock/platforms without LEDs.
    fn led_event_sender(&self) -> Option<tokio::sync::mpsc::Sender<LedEvent>>;

    // ── BOS firmware upgrade ─────────────────────────────────────

    /// Run BOS firmware sysupgrade with the given image file.
    /// This reboots the device — the function does not return on success.
    /// When `keep_settings` is `false`, pass `-n` to sysupgrade (no save).
    fn bos_upgrade(
        &self,
        image_path: &std::path::Path,
        keep_settings: bool,
    ) -> impl std::future::Future<Output = Result<(), InitError>> + Send;

    /// Get the device platform identifier for firmware index lookup.
    fn platform(&self) -> bmc_platform::BosPlatform;
}

/// Check if activation already ran this boot.
///
/// The volatile sentinel `/tmp/nix_activated` is written by the
/// `999-activated` activation script. It lives on tmpfs so it is
/// cleared on every reboot.
#[must_use]
pub fn is_activated_this_boot(config: &InitConfig) -> bool {
    config.activation_sentinel.exists()
}

/// How long to wait for a configured STA network to connect on boot.
pub(crate) const WIFI_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Poll interval while waiting for WiFi connection.
pub(crate) const WIFI_CONNECT_POLL: std::time::Duration = std::time::Duration::from_secs(2);

/// Send an LED event if the platform has LED hardware.
async fn send_led_event<P: InitPlatform>(platform: &P, event: LedEvent) {
    if let Some(tx) = platform.led_event_sender() {
        let _ = tx.send(event).await;
    }
}

/// Wait for an already-configured STA WiFi to obtain an IP address,
/// polling at `poll` interval up to `timeout`. Returns `Ok(true)` if
/// an IP was assigned, `Ok(false)` if timed out.
async fn wait_for_wifi_connection<P: InitPlatform>(
    platform: Arc<P>,
    timeout: std::time::Duration,
    poll: std::time::Duration,
) -> Result<bool, InitError> {
    tracing::info!(
        "WiFi STA configured, waiting up to {}s for IP address",
        timeout.as_secs()
    );
    let deadline = tokio::time::Instant::now() + timeout;
    let mut interval = tokio::time::interval(poll);
    loop {
        interval.tick().await;
        if platform.ip_address().await.is_some() {
            tracing::info!("WiFi connected, IP address obtained");
            return Ok(true);
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!("WiFi connection timed out after {}s", timeout.as_secs());
            return Ok(false);
        }
    }
}

/// Guard that aborts a spawned task when dropped.
///
/// Prevents the HTTP server from leaking and holding port 80 if
/// `wait_for_wifi_setup` exits via an error path.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Run the WiFi AP mode setup: start server, wait for WiFi connection,
/// then shut down server and captive portal.
///
/// This is extracted so it can be reused both for the initial AP setup
/// and for re-entering AP mode after a network error.
#[expect(
    clippy::too_many_lines,
    reason = "linear WiFi setup flow; splitting would harm readability"
)]
async fn wait_for_wifi_setup<P: InitPlatform + 'static>(
    config: &InitConfig,
    platform: Arc<P>,
    observer: &dyn InitStateObserver,
) -> Result<(), InitError> {
    let mut ap_ssid = platform.configure_wifi_ap().await?;
    platform.enable_captive_portal().await?;
    send_led_event(&*platform, LedEvent::WifiScan).await;
    observer.on_state_change(&InitState::NoWifi {
        ap_ssid: ap_ssid.clone(),
    });

    let (wifi_tx, mut wifi_rx) = tokio::sync::mpsc::channel::<Result<(), String>>(1);
    let (state_tx, mut state_rx) = tokio::sync::mpsc::channel::<InitState>(4);
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let mut server_handle = AbortOnDrop(tokio::spawn(crate::server::run_wifi_setup_server(
        platform.clone(),
        config.www_path.clone(),
        std::net::SocketAddr::from(([0, 0, 0, 0], 80)),
        shutdown_rx,
        wifi_tx,
        state_tx,
    )));

    let mut last_connecting_ssid = String::new();
    let mut ssid_refresh = tokio::time::interval(std::time::Duration::from_secs(5));
    ssid_refresh.tick().await;
    loop {
        tokio::select! {
            wifi_result = wifi_rx.recv() => {
                match wifi_result {
                    Some(Ok(())) => {
                        tracing::info!("WiFi connected (notified by SetWifi handler)");
                        break;
                    }
                    Some(Err(msg)) => {
                        tracing::error!("WiFi connection failed: {msg}, reverting to AP mode");

                        send_led_event(&*platform, LedEvent::WifiError).await;
                        observer.on_state_change(&InitState::ConnectionFailed {
                            ssid: last_connecting_ssid.clone(),
                        });

                        // Show failure screen for 3 seconds before reverting
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

                        ap_ssid = platform.configure_wifi_ap().await?;
                        platform.enable_captive_portal().await?;
                        observer.on_state_change(&InitState::NoWifi {
                            ap_ssid: ap_ssid.clone(),
                        });
                        continue;
                    }
                    None => {
                        return Err(InitError::wifi(
                            "WiFi setup channel closed unexpectedly",
                        ));
                    }
                }
            }
            state = state_rx.recv() => {
                if let Some(ref state) = state {
                    if let InitState::Connecting { ssid } = state {
                        last_connecting_ssid.clone_from(ssid);
                    }
                    observer.on_state_change(state);
                }
            }
            result = &mut server_handle.0 => {
                match result {
                    Ok(Ok(())) => {
                        return Err(InitError::wifi(
                            "WiFi setup server exited unexpectedly",
                        ));
                    }
                    Ok(Err(e)) => {
                        return Err(InitError::wifi(format!(
                            "WiFi setup server error: {e}",
                        )));
                    }
                    Err(e) => {
                        return Err(InitError::wifi(format!(
                            "WiFi setup server task panicked: {e}",
                        )));
                    }
                }
            }
            _ = ssid_refresh.tick() => {
                if let Some(current) = platform.wifi_ap_ssid().await
                    && current != ap_ssid
                {
                    tracing::info!("AP SSID changed: {ap_ssid} → {current}");
                    ap_ssid = current;
                    observer.on_state_change(&InitState::NoWifi {
                        ap_ssid: ap_ssid.clone(),
                    });
                }
            }
        }
    }

    send_led_event(&*platform, LedEvent::WifiScanEnded).await;
    send_led_event(&*platform, LedEvent::WifiConnected).await;

    let _ = shutdown_tx.send(());
    if let Err(e) = platform.disable_captive_portal().await {
        tracing::warn!("failed to disable captive portal (non-fatal): {e}");
    }
    let connected_ssid = platform.wifi_sta_ssid().await.unwrap_or_default();
    observer.on_state_change(&InitState::Connecting {
        ssid: connected_ssid,
    });

    Ok(())
}

/// Build a shared HTTP client.
///
/// TLS cert validation is disabled because NTP has not synced on
/// first boot (clock is at epoch → certs appear "not yet valid").
/// Tarball integrity is ensured by signature verification, not TLS.
fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .danger_accept_invalid_certs(true)
        .build()
        .expect("BUG: failed to build HTTP client")
}

/// Execute the store initialization and activation steps (4-9).
///
/// This covers: read config, fetch index, download, staged extraction,
/// mount store, activate, set setup pending, and set init marker.
///
/// `wipe_store` is forwarded to `store::init_store`. Pass `true` on the
/// first attempt and `false` on subsequent retries so the store is not
/// wiped unnecessarily.
async fn init_store_and_activate<P: InitPlatform + 'static>(
    config: &InitConfig,
    platform: Arc<P>,
    observer: &dyn InitStateObserver,
    client: &reqwest::Client,
    wipe_store: bool,
) -> Result<(), InitError> {
    let servers_config = bmc_nix::servers_config::load_servers_config(&config.servers_config_path)
        .map_err(|err| InitError::config(err.to_string()))?;
    let bos_version = config
        .read_bos_version()
        .map_err(|e| InitError::config(format!("failed to read BOS version: {e}")))?;

    send_led_event(&*platform, LedEvent::WifiScanEnded).await;
    observer.on_state_change(&InitState::FetchingIndex);

    let progress_observer = DownloadProgressAdapter::new(observer);
    let init_result = store::init_store(
        client,
        &servers_config.factory,
        &bos_version,
        &config.download_dir,
        &config.nix_stage_dir(),
        wipe_store,
        Some(&progress_observer),
    )
    .await
    .map_err(InitError::Store)?;

    platform.mount_nix_store(config).await?;

    observer.on_state_change(&InitState::Activating);
    platform
        .activate_generation(&init_result.profile_path)
        .await?;

    platform.set_setup_pending()?;
    platform.set_init_marker(config)?;
    observer.on_state_change(&InitState::Done);

    Ok(())
}

/// Run the full initialization flow.
///
/// Flow:
/// 1. Check if already activated this boot → exit
/// 3. Check WiFi connectivity, enter AP mode if needed
/// 4. Fetch factory index, download and extract tarball
/// 5. Bind mount the promoted store
/// 6. Activate initial profile
/// 7. Set persistent init marker
///
/// If a network-related error occurs during steps 4-9 and the user
/// presses "Reconfigure WiFi", the flow re-enters AP mode and retries.
#[expect(
    clippy::too_many_lines,
    reason = "linear init flow; splitting would harm readability"
)]
pub async fn run_init<P: InitPlatform + 'static>(
    config: &InitConfig,
    platform: Arc<P>,
    observer: &dyn InitStateObserver,
    mut reconfigure_rx: tokio::sync::mpsc::UnboundedReceiver<()>,
    mut retry_rx: tokio::sync::mpsc::UnboundedReceiver<()>,
) -> Result<(), InitError> {
    tracing::info!("checking store initialization status");

    // Check inhibit file first — if present, skip init entirely
    if config.inhibit_init_path.exists() {
        tracing::info!("init inhibited by {}", config.inhibit_init_path.display());
        return Ok(());
    }

    // TODO: activation sentinel check disabled — store existence check
    // is now the primary gate. Re-evaluate how to make this safe.
    // if is_activated_this_boot(config) {
    //     tracing::info!("already activated this boot, exiting");
    //     return Ok(());
    // }

    if platform.is_store_ever_initialized(config) {
        tracing::info!("store already initialized, exiting");
        return Ok(());
    }

    // Skip AP mode if the device already has any usable IP (Ethernet or a
    // pre-associated WiFi STA). `ip_address()` matches any running,
    // non-loopback interface with an IPv4 address, so wired connectivity
    // short-circuits the WiFi flow.
    if platform.ip_address().await.is_none() {
        let need_ap = if platform.has_wifi_configuration().await? {
            // STA is configured — wait for it to associate before falling back to AP.
            let sta_ssid = platform.wifi_sta_ssid().await.unwrap_or_default();
            observer.on_state_change(&InitState::Connecting { ssid: sta_ssid });
            !wait_for_wifi_connection(platform.clone(), WIFI_CONNECT_TIMEOUT, WIFI_CONNECT_POLL)
                .await?
        } else {
            true
        };
        if need_ap {
            wait_for_wifi_setup(config, platform.clone(), observer).await?;
        }
    } else {
        tracing::info!("network already online, skipping WiFi setup");
    }
    let client = build_http_client();
    let mut store_ever_attempted = false;
    loop {
        let wipe_store = !store_ever_attempted;
        store_ever_attempted = true;
        match init_store_and_activate(config, platform.clone(), observer, &client, wipe_store).await
        {
            Ok(()) => break,
            Err(e) if e.is_bos_version_mismatch() => {
                tracing::warn!(
                    "no factory tarball for current BOS version, attempting firmware upgrade"
                );
                let bos_version = config
                    .read_bos_version()
                    .unwrap_or_else(|_| "unknown".into());

                match crate::bos_upgrade::try_bos_upgrade(
                    &client,
                    &*platform,
                    observer,
                    &bos_version,
                    &config.download_dir,
                    config.keep_settings,
                )
                .await
                {
                    Ok(true) => {
                        tracing::info!("BOS upgrade triggered, awaiting reboot");
                        return Ok(());
                    }
                    Ok(false) => {
                        return Err(InitError::config(format!(
                            "no factory tarball for BOS version '{bos_version}' \
                             and no firmware upgrade available"
                        )));
                    }
                    Err(upgrade_err) => {
                        tracing::error!("BOS upgrade failed: {upgrade_err}");
                        observer.on_state_change(&InitState::Error {
                            message: format!("Firmware upgrade failed: {upgrade_err}"),
                            retryable: true,
                            reconfigurable: true,
                        });
                        // Drain stale messages before waiting
                        while retry_rx.try_recv().is_ok() {}
                        while reconfigure_rx.try_recv().is_ok() {}
                        tokio::select! {
                            _ = retry_rx.recv() => continue,
                            _ = reconfigure_rx.recv() => {
                                wait_for_wifi_setup(config, platform.clone(), observer).await?;
                                continue;
                            }
                        }
                    }
                }
            }
            Err(e) if e.is_network_related() => {
                tracing::error!("init failed: {e}, waiting for retry or reconfigure");
                observer.on_state_change(&InitState::Error {
                    message: e.to_string(),
                    retryable: true,
                    reconfigurable: true,
                });

                // Wait for user to press either "Retry download" or
                // "Reconfigure WiFi".
                // Drain stale messages before waiting
                while retry_rx.try_recv().is_ok() {}
                while reconfigure_rx.try_recv().is_ok() {}
                tokio::select! {
                    _ = retry_rx.recv() => {
                        // Retry the download directly without re-entering AP mode
                        tracing::info!("retrying download");
                        continue;
                    }
                    _ = reconfigure_rx.recv() => {
                        // Re-enter AP mode and wait for new WiFi connection
                        tracing::info!("re-entering AP mode for WiFi reconfiguration");
                        wait_for_wifi_setup(config, platform.clone(), observer).await?;
                        continue;
                    }
                }
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

/// Adapter to bridge `store::DownloadProgress` to `InitStateObserver`.
struct DownloadProgressAdapter<'a> {
    observer: &'a dyn InitStateObserver,
    last_update: std::sync::Mutex<std::time::Instant>,
}

const DOWNLOAD_PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(300);

impl<'a> DownloadProgressAdapter<'a> {
    fn new(observer: &'a dyn InitStateObserver) -> Self {
        Self {
            observer,
            last_update: std::sync::Mutex::new(
                std::time::Instant::now()
                    .checked_sub(DOWNLOAD_PROGRESS_INTERVAL)
                    .unwrap_or_else(std::time::Instant::now),
            ),
        }
    }
}

impl store::DownloadProgress for DownloadProgressAdapter<'_> {
    fn on_bytes_downloaded(&self, downloaded: usize, total: Option<usize>) {
        let mut last = self
            .last_update
            .lock()
            .expect("BUG: progress throttle mutex poisoned");
        if last.elapsed() < DOWNLOAD_PROGRESS_INTERVAL {
            return;
        }
        *last = std::time::Instant::now();
        self.observer.on_state_change(&InitState::Downloading {
            downloaded_bytes: downloaded,
            total_bytes: total,
        });
    }
    fn on_extracting(&self) {
        self.observer.on_state_change(&InitState::Extracting);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error("configuration error: {message}")]
    Config {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
    #[error("store initialization failed: {0}")]
    Store(#[from] store::InitStoreError),
    #[error("activation failed: {message}")]
    Activation {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
    #[error("WiFi error: {0}")]
    Wifi(String),
    #[error("network error: {0}")]
    Network(String),
}

impl InitError {
    /// Returns `true` if the error is a BOS version mismatch (no factory
    /// tarball for this BOS version). The init binary should attempt a BOS
    /// firmware upgrade in this case.
    #[must_use]
    pub fn is_bos_version_mismatch(&self) -> bool {
        matches!(
            self,
            Self::Store(store::InitStoreError::MissingBosVersion(_))
        )
    }

    /// Returns `true` for errors where retrying or reconfiguring WiFi
    /// could help (network / download / WiFi failures). Local errors
    /// like disk-full or missing BOS version are not network-related.
    #[must_use]
    pub fn is_network_related(&self) -> bool {
        match self {
            Self::Wifi(_) | Self::Network(_) => true,
            Self::Store(e) => matches!(
                e,
                store::InitStoreError::FactoryIndexFetch { .. }
                    | store::InitStoreError::FactoryIndexParse { .. }
                    | store::InitStoreError::DownloadFailed { .. }
                    | store::InitStoreError::DownloadStalled { .. }
            ),
            Self::Config { .. } | Self::Activation { .. } => false,
        }
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::Config {
            message: message.into(),
            source: None,
        }
    }

    pub fn activation(message: impl Into<String>) -> Self {
        Self::Activation {
            message: message.into(),
            source: None,
        }
    }

    pub fn wifi(message: impl Into<String>) -> Self {
        Self::Wifi(message.into())
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self::Network(message.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct TestPlatform {
        store_initialized: bool,
    }

    impl InitPlatform for TestPlatform {
        fn is_store_ever_initialized(&self, _config: &InitConfig) -> bool {
            self.store_initialized
        }
        fn set_init_marker(&self, _config: &InitConfig) -> Result<(), InitError> {
            Ok(())
        }
        async fn mount_nix_store(&self, _config: &InitConfig) -> Result<(), InitError> {
            Ok(())
        }
        async fn activate_generation(&self, _profile_dir: &Path) -> Result<(), InitError> {
            Ok(())
        }
        fn set_setup_pending(&self) -> Result<(), InitError> {
            Ok(())
        }
        async fn has_wifi_configuration(&self) -> Result<bool, InitError> {
            Ok(true)
        }
        async fn ip_address(&self) -> Option<std::net::IpAddr> {
            Some(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
        }
        async fn configure_wifi_ap(&self) -> Result<String, InitError> {
            Ok("TestAP".into())
        }
        async fn enable_captive_portal(&self) -> Result<(), InitError> {
            Ok(())
        }
        async fn disable_captive_portal(&self) -> Result<(), InitError> {
            Ok(())
        }
        async fn scan_wifi(&self) -> Result<Vec<WifiScanItem>, InitError> {
            Ok(vec![])
        }
        async fn save_and_connect(
            &self,
            _ssid: String,
            _password: Option<String>,
            _encryption: EncryptionType,
        ) -> Result<(), InitError> {
            Ok(())
        }
        async fn wifi_ap_ssid(&self) -> Option<String> {
            Some("TestAP".to_owned())
        }
        async fn wifi_sta_ssid(&self) -> Option<String> {
            Some("TestSTA".to_owned())
        }
        fn led_event_sender(&self) -> Option<tokio::sync::mpsc::Sender<LedEvent>> {
            None
        }
        async fn bos_upgrade(
            &self,
            _image_path: &std::path::Path,
            _keep_settings: bool,
        ) -> Result<(), InitError> {
            Ok(())
        }
        fn platform(&self) -> bmc_platform::BosPlatform {
            bmc_platform::BosPlatform::Bmc1
        }
    }

    struct NoopObserver;
    impl InitStateObserver for NoopObserver {
        fn on_state_change(&self, _state: &InitState) {}
    }

    #[test]
    fn is_activated_this_boot_detects_sentinel() {
        let tmp = TempDir::new().expect("BUG: temp dir");
        let config = InitConfig {
            activation_sentinel: tmp.path().join("nix_activated"),
            ..Default::default()
        };
        assert!(!is_activated_this_boot(&config));
        std::fs::write(&config.activation_sentinel, "1").expect("BUG: write");
        assert!(is_activated_this_boot(&config));
    }

    #[tokio::test]
    async fn run_init_exits_early_when_inhibit_file_exists() {
        let tmp = TempDir::new().expect("BUG: temp dir");
        let config = InitConfig {
            inhibit_init_path: tmp.path().join("NIX_INHIBIT_INIT"),
            ..Default::default()
        };
        // Write inhibit file — init should exit immediately
        std::fs::write(&config.inhibit_init_path, "1").expect("BUG: write");

        let platform = Arc::new(TestPlatform {
            store_initialized: false,
        });
        let (_reconfigure_tx, reconfigure_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_retry_tx, retry_rx) = tokio::sync::mpsc::unbounded_channel();
        let result = run_init(&config, platform, &NoopObserver, reconfigure_rx, retry_rx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn run_init_exits_early_when_store_initialized() {
        let tmp = TempDir::new().expect("BUG: temp dir");
        let config = InitConfig {
            activation_sentinel: tmp.path().join("nix_activated"),
            ..Default::default()
        };

        let platform = Arc::new(TestPlatform {
            store_initialized: true,
        });
        let (_reconfigure_tx, reconfigure_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_retry_tx, retry_rx) = tokio::sync::mpsc::unbounded_channel();
        let result = run_init(&config, platform, &NoopObserver, reconfigure_rx, retry_rx).await;
        assert!(result.is_ok());
    }
}
