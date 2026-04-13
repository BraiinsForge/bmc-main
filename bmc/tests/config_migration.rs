// Copyright (C) 2025  Braiins Systems s.r.o.

//! Integration test for `bmc::config_migration` driven by a real
//! config captured from a device running the old slint-monolith
//! firmware.
//!
//! The fixture lives at `bmc/tests/fixtures/legacy_config_sample.json`
//! and must only be modified when we capture a new device snapshot.

use std::path::PathBuf;

use bmc::config_migration::{self, Report};
use tokio::fs;

const FIXTURE: &str = include_str!("fixtures/legacy_config_sample.json");

#[tokio::test]
async fn migrates_device_sample_without_losing_scenes() {
    let tmp = tempdir();
    let dest = tmp.join("bmc_config.json");
    fs::write(&dest, FIXTURE)
        .await
        .expect("BUG: seed fixture write should succeed");

    let report = config_migration::migrate_in_place(&dest)
        .await
        .expect("BUG: migration of captured device sample should succeed");

    assert!(
        report.was_legacy,
        "device sample must trigger the legacy path"
    );
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

    let migrated = fs::read_to_string(&dest)
        .await
        .expect("BUG: migrated file should be readable");
    let v: serde_json::Value =
        serde_json::from_str(&migrated).expect("BUG: migrated output must be valid JSON");

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
                // Unavailable placeholders must keep the original data
                // around so a later migration pass can promote them.
                let legacy = &widget["params"]["_legacy"];
                assert!(
                    legacy.is_object(),
                    "unavailable placeholder missing _legacy payload: {widget}",
                );
                assert!(legacy["kind"].is_string());
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
async fn already_new_config_is_a_noop() {
    let tmp = tempdir();
    let dest = tmp.join("bmc_config.json");
    // A minimal new-shape config: top-level keys that serde-default
    // into Config with empty scenes.
    fs::write(&dest, r#"{"scenes":[]}"#)
        .await
        .expect("BUG: seed write should succeed");

    let report: Report = config_migration::migrate_in_place(&dest)
        .await
        .expect("BUG: no-op migration should succeed");

    assert!(!report.was_legacy, "new-format config must be detected");
    assert_eq!(report.scenes, 0);
    assert_eq!(report.translated_widgets, 0);
    assert_eq!(report.unavailable_widgets, 0);

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
