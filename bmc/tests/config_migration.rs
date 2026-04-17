// Copyright (C) 2025  Braiins Systems s.r.o.

//! Integration test for `bmc::config_migration` driven by a real
//! config captured from a device running the old slint-monolith
//! firmware.
//!
//! The fixture lives at `bmc/tests/fixtures/legacy_config_sample.json`
//! and must only be modified when we capture a new device snapshot.

use std::path::PathBuf;
use std::str::FromStr;

use bmc::config_migration::{self, LoadedConfig};
use tokio::fs;

const FIXTURE: &str = include_str!("fixtures/legacy_config_sample.json");

#[tokio::test]
async fn migrates_device_sample_without_losing_scenes() {
    let tmp = tempdir();
    let dest = tmp.join("bmc_config.json");
    fs::write(&dest, FIXTURE)
        .await
        .expect("BUG: seed fixture write should succeed");

    let loaded = config_migration::migrate_on_disk(&dest)
        .await
        .expect("BUG: migration of captured device sample should succeed");

    assert!(
        loaded.was_migrated(),
        "device sample must trigger the legacy path"
    );

    let report = loaded
        .report()
        .expect("BUG: migrated load must carry a report");
    assert_eq!(
        report.scenes, 3,
        "fixture has 3 scenes and they must all survive",
    );
    assert!(
        report.translated_widgets >= 1,
        "at least the digital clock must translate (got {})",
        report.translated_widgets,
    );
    assert!(
        report.unavailable_widgets >= 1,
        "fixture contains widgets with no manifest (ticker_btc, block_height)",
    );

    // The preserved v0 struct must still be available after the
    // upgrade. This is the core "in-memory migration" property —
    // nothing else needs to touch the disk to see what the config
    // looked like before the rewrite.
    let original = loaded
        .original_v0()
        .expect("BUG: migrated load must preserve the v0 struct");
    assert_eq!(original.scenes.len(), 3);

    // And the upgrade must have been serialised back to disk with
    // the current schema version.
    let migrated = fs::read_to_string(&dest)
        .await
        .expect("BUG: migrated file should be readable");
    let v: serde_json::Value =
        serde_json::from_str(&migrated).expect("BUG: migrated output must be valid JSON");
    assert_eq!(
        v["version"], 1,
        "migrated config must carry the current schema version",
    );

    let scenes = v["scenes"]
        .as_array()
        .expect("BUG: migrated config must have a scenes array");
    assert_eq!(scenes.len(), 3);

    for scene in scenes {
        for widget in scene["widgets"]
            .as_array()
            .expect("BUG: each scene must have a widgets array")
        {
            let type_id = widget["widget_type_id"]
                .as_str()
                .expect("BUG: widget_type_id must be a string");
            if type_id == "00000000-0000-0000-0000-000000000000" {
                // Placeholders must preserve enough legacy data for a
                // later migration pass to promote them. Two accepted
                // shapes: `_legacy` (kind + params) for native widgets,
                // `_legacy_remote` (name, url, icon, ...) for the old
                // remote widget.
                let params = &widget["params"];
                let has_legacy = params["_legacy"].is_object();
                let has_legacy_remote = params["_legacy_remote"].is_object();
                assert!(
                    has_legacy || has_legacy_remote,
                    "placeholder missing _legacy or _legacy_remote payload: {widget}",
                );
                if has_legacy {
                    assert!(params["_legacy"]["kind"].is_string());
                }
                if has_legacy_remote {
                    assert!(params["_legacy_remote"]["widget_url"].is_string());
                }
            }
        }
    }

    // Backup must exist next to dest.
    let parent = dest
        .parent()
        .expect("BUG: tmp dir path always has a parent");
    let mut entries = fs::read_dir(parent)
        .await
        .expect("BUG: tmp dir must be readable");
    let mut saw_backup = false;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("bmc_config.json.backup.") {
            saw_backup = true;
            break;
        }
    }
    assert!(saw_backup, "backup file must be written");
}

#[tokio::test]
async fn current_version_config_is_a_noop() {
    let tmp = tempdir();
    let dest = tmp.join("bmc_config.json");
    fs::write(&dest, r#"{"version":1,"scenes":[],"accounts":[]}"#)
        .await
        .expect("BUG: seed write should succeed");

    let loaded = config_migration::migrate_on_disk(&dest)
        .await
        .expect("BUG: no-op migration should succeed");

    assert!(
        !loaded.was_migrated(),
        "already-versioned config must be a no-op"
    );
    assert!(loaded.original_v0().is_none());
    assert!(loaded.report().is_none());

    // Backup must NOT be written on a no-op.
    let parent = dest
        .parent()
        .expect("BUG: tmp dir path always has a parent");
    let mut entries = fs::read_dir(parent)
        .await
        .expect("BUG: tmp dir must be readable");
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(
            !name.contains(".backup."),
            "no-op must not create a backup (got {name})",
        );
    }
}

#[tokio::test]
async fn unknown_future_version_is_rejected() {
    let tmp = tempdir();
    let dest = tmp.join("bmc_config.json");
    fs::write(&dest, r#"{"version":999,"scenes":[]}"#)
        .await
        .expect("BUG: seed write should succeed");

    let err = config_migration::migrate_on_disk(&dest)
        .await
        .expect_err("future version must be refused rather than overwritten");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("unsupported config version"),
        "error should name the failure mode (got: {msg})",
    );

    // File must not have been modified.
    let on_disk = fs::read_to_string(&dest)
        .await
        .expect("BUG: original file should still be readable");
    assert!(on_disk.contains("999"), "file must be untouched");
}

#[tokio::test]
async fn load_is_pure_without_persist() {
    // `LoadedConfig::from_str` is the pure, in-memory path. No
    // filesystem side effects; the caller is free to inspect the
    // result and discard it.
    let loaded =
        LoadedConfig::from_str(FIXTURE).expect("BUG: fixture must parse via the pure FromStr path");
    assert!(loaded.was_migrated());
    assert!(
        loaded.original_v0().is_some(),
        "FromStr path must also preserve the v0 original"
    );
}

/// Small helper producing a unique tmp dir for each test.
fn tempdir() -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "bmc-migration-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    std::fs::create_dir_all(&base).expect("BUG: tmp dir creation should succeed");
    base
}
