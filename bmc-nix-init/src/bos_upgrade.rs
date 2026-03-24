// Copyright (C) 2026  Braiins Systems s.r.o.

use std::path::Path;
use std::time::{Duration, Instant};

use bmc_upgrade::bmc_index::BmcIndex;
use bmc_upgrade::upgrader::{DownloadState, FirmwareUpgrader};

use crate::init::{InitError, InitPlatform};
use crate::state::{InitState, InitStateObserver};

const UPDATE_PROGRESS_INTERVAL: Duration = Duration::from_millis(300);

/// Check for a BOS firmware upgrade and perform it if available.
///
/// Uses `FirmwareUpgrader` with `BmcIndex` to find the next upgrade step.
/// This respects major version boundaries — the device upgrades one step
/// at a time and reboots between each major version.
///
/// Returns `Ok(true)` if sysupgrade was triggered (device will reboot),
/// `Ok(false)` if no upgrade is available, or `Err` on failure.
pub async fn try_bos_upgrade<P: InitPlatform + 'static>(
    client: &reqwest::Client,
    platform: &P,
    observer: &dyn InitStateObserver,
    bos_version_full: &str,
    download_dir: &Path,
    keep_settings: bool,
) -> Result<bool, InitError> {
    tracing::info!("checking for BOS firmware upgrade (current: {bos_version_full})");

    let image_path = download_dir.join("sysupgrade.bin");
    let upgrader = FirmwareUpgrader::new(BmcIndex::default(), image_path.clone(), client.clone());

    let upgrade_detail = upgrader
        .check_for_upgrade(platform.platform(), bos_version_full.to_owned())
        .await
        .map_err(|e| InitError::network(format!("failed to check for firmware upgrade: {e}")))?;

    let Some(detail) = upgrade_detail else {
        tracing::info!("no firmware upgrade available");
        return Ok(false);
    };

    let release = &detail.latest_release;
    tracing::info!(
        "upgrading BOS from {} to {} ({})",
        bos_version_full,
        release.version,
        release.url,
    );

    #[expect(clippy::cast_precision_loss)]
    let total_mb = release.file_size as f32 / 1_000_000.0;

    observer.on_state_change(&InitState::UpgradingFirmware {
        downloaded_mb: 0.0,
        total_mb,
    });

    let mut rx = upgrader.download_firmware(
        release.url.clone(),
        release.hash.clone(),
        release.file_size as u64,
    );

    let mut progress_updated_at = Instant::now();
    while let Some(state) = rx.recv().await {
        match state {
            DownloadState::Progress {
                downloaded_mb,
                total_mb,
            } => {
                if progress_updated_at.elapsed() < UPDATE_PROGRESS_INTERVAL {
                    continue;
                }
                progress_updated_at = Instant::now();
                observer.on_state_change(&InitState::UpgradingFirmware {
                    downloaded_mb,
                    total_mb,
                });
            }
            DownloadState::Finished { hash } => {
                tracing::info!("firmware download complete, hash: {hash}");
            }
            DownloadState::Failed(e) => {
                let _ = tokio::fs::remove_file(&image_path).await;
                return Err(InitError::network(format!("firmware download failed: {e}")));
            }
        }
    }

    // Re-verify the downloaded image from disk
    upgrader
        .verify_firmware(&release.hash)
        .await
        .map_err(|e| InitError::config(format!("firmware verification failed: {e}")))?;

    // Run sysupgrade — device reboots, init re-runs with new BOS version
    observer.on_state_change(&InitState::Rebooting);
    platform.bos_upgrade(&image_path, keep_settings).await?;

    Ok(true)
}
