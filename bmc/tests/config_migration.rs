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

//! Integration test for `bmc::config_migration` driven by a real
//! config captured from a device running the old slint-monolith
//! firmware.
//!
//! The fixture lives at `bmc/tests/fixtures/legacy_config_sample.json`
//! and must only be modified when we capture a new device snapshot.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use bmc::config_migration::{self, CONFIG_VERSION, LoadedConfig};
use tempfile::TempDir;
use tokio::fs;

const FIXTURE: &str = include_str!("fixtures/legacy_config_sample.json");

const NIL_UUID: &str = "00000000-0000-0000-0000-000000000000";

/// Load, upgrade if needed, validate, and persist in place. Mirrors the
/// offline `bmc-migrate-config` chain (load → validate → `save_with_backup`);
/// the boot path deliberately does *not* persist (migration is in-memory,
/// committed only on the first genuine change). Kept here to exercise the
/// persist chain against captured samples.
async fn migrate_and_persist(path: &Path) -> Result<LoadedConfig> {
    let loaded = config_migration::load_any_version(path).await?;
    if loaded.was_migrated() {
        loaded
            .validate()
            .context("migrated config failed validation")?;
        config_migration::save_with_backup(loaded.current(), path).await?;
    }
    Ok(loaded)
}

#[tokio::test]
async fn migrates_device_sample_without_losing_scenes() {
    let tmp = tempdir();
    let dest = tmp.path().join("bmc_config.json");
    fs::write(&dest, FIXTURE)
        .await
        .expect("BUG: seed fixture write should succeed");

    let loaded = migrate_and_persist(&dest)
        .await
        .expect("BUG: migration of captured device sample should succeed");

    assert!(
        loaded.was_migrated(),
        "device sample must trigger the legacy path"
    );

    let report = loaded
        .report()
        .expect("BUG: migrated load must carry a report");
    // The fixture has 3 scenes; the ticker-only scene loses its sole
    // widget (`ticker_btc` has no native counterpart) and drops with
    // it, per the no-placeholders policy.
    assert_eq!(report.scenes, 2, "both scenes with survivors must stay");
    assert_eq!(report.dropped_scenes, 1, "the ticker-only scene drops");
    assert_eq!(
        report.translated_widgets, 3,
        "both clocks and the block height must translate",
    );
    assert_eq!(
        report.dropped_widgets, 2,
        "both ticker_btc widgets must drop",
    );

    // The number of widgets on disk equals translated (dropped
    // widgets do not appear in the output).
    let migrated = fs::read_to_string(&dest)
        .await
        .expect("BUG: migrated file should be readable");
    let v: serde_json::Value =
        serde_json::from_str(&migrated).expect("BUG: migrated output must be valid JSON");
    assert_eq!(
        v["version"], CONFIG_VERSION,
        "migrated config must carry the current schema version",
    );

    let scenes = v["scenes"]
        .as_array()
        .expect("BUG: migrated config must have a scenes array");
    assert_eq!(scenes.len(), 2, "the dropped scene must not be on disk");

    // Post-upgrade invariant: every widget carries a reserved
    // (non-nil) UID. No placeholder widgets are allowed.
    let mut on_disk_widget_count = 0_usize;
    for scene in scenes {
        for widget in scene["widgets"]
            .as_array()
            .expect("BUG: each scene must have a widgets array")
        {
            on_disk_widget_count += 1;
            let type_id = widget["widget_type_id"]
                .as_str()
                .expect("BUG: widget_type_id must be a string");
            assert_ne!(
                type_id, "00000000-0000-0000-0000-000000000000",
                "post-upgrade widgets must never have a nil widget_type_id: {widget}",
            );
            // Upgraded widgets must not carry the retired `_legacy` /
            // `_legacy_remote` placeholder shape either.
            let params = &widget["params"];
            assert!(
                params.get("_legacy").is_none() && params.get("_legacy_remote").is_none(),
                "retired placeholder shape leaked into an upgraded widget: {widget}",
            );
        }
    }
    assert_eq!(
        on_disk_widget_count, report.translated_widgets,
        "on-disk widget count must match the translated counter"
    );

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

/// Golden migration snapshots: `<case>.v0.json` migrates to
/// `<case>.expected.json` plus `<case>.accounts.json`. Widening the suite
/// to another widget is three files, no new test code.
const GOLDEN_DIR: &str = "tests/fixtures/migration";

/// Rewrite the expected sides instead of comparing, for when a migration
/// change is meant to move them. The diff is then the review artifact —
/// as with netsim's `UPDATE_SCHEMA`.
fn blessing() -> bool {
    std::env::var_os("UPDATE_GOLDEN").is_some()
}

/// Every `<case>.v0.json` under [`GOLDEN_DIR`], by stem.
fn golden_cases() -> Vec<(String, PathBuf)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_DIR);
    let mut cases: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("BUG: {} must be readable: {e}", dir.display()))
        .filter_map(|entry| {
            let path = entry.expect("BUG: readable dir entry").path();
            let stem = path.file_name()?.to_str()?.strip_suffix(".v0.json")?;
            Some((stem.to_owned(), path))
        })
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "the golden suite must have a case");
    cases
}

fn compare_or_bless(path: &Path, actual: &serde_json::Value, case: &str) {
    let rendered = format!(
        "{}\n",
        serde_json::to_string_pretty(actual).expect("BUG: migrated value must render")
    );
    if blessing() {
        std::fs::write(path, &rendered)
            .unwrap_or_else(|e| panic!("BUG: blessing {} must write: {e}", path.display()));
        return;
    }
    let expected = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "BUG: {} missing ({e}); create it with UPDATE_GOLDEN=1",
            path.display()
        )
    });
    // `similar_asserts` prints the differing hunks; the std macro would
    // escape both whole files onto one line each.
    similar_asserts::assert_eq!(
        expected: expected,
        actual: rendered,
        "{case}: migration output drifted from {}; \
         review the diff, then re-bless with UPDATE_GOLDEN=1",
        path.display(),
    );
}

/// The migration is a pure function of its input — ids come from the
/// legacy ids and nothing reads the clock — so its whole output is
/// comparable, not just the fields a hand-written assertion remembers.
#[test]
fn golden_snapshots_migrate_to_their_expected_config_and_accounts() {
    for (case, input) in golden_cases() {
        let raw = std::fs::read_to_string(&input).expect("BUG: fixture must be readable");
        let loaded = LoadedConfig::from_str(&raw)
            .unwrap_or_else(|e| panic!("BUG: {case} must migrate: {e:#}"));
        assert!(loaded.was_migrated(), "{case}: fixture must be legacy");
        loaded
            .validate()
            .unwrap_or_else(|e| panic!("BUG: {case} must migrate to a valid config: {e:#}"));

        let dir = input.parent().expect("BUG: fixture path has a parent");
        compare_or_bless(
            &dir.join(format!("{case}.expected.json")),
            &serde_json::to_value(loaded.current()).expect("BUG: config must serialize"),
            &case,
        );
        compare_or_bless(
            &dir.join(format!("{case}.accounts.json")),
            &serde_json::to_value(loaded.extracted_accounts())
                .expect("BUG: accounts must serialize"),
            &case,
        );
    }
}

/// Properties no re-blessing can paper over: a golden file records what
/// the migration *does*, these record what it *owes*.
#[test]
fn migrated_widgets_honour_their_manifests_and_bindings() {
    for (case, input) in golden_cases() {
        let raw = std::fs::read_to_string(&input).expect("BUG: fixture must be readable");
        let loaded = LoadedConfig::from_str(&raw)
            .unwrap_or_else(|e| panic!("BUG: {case} must migrate: {e:#}"));
        let migrated = serde_json::to_value(loaded.current()).expect("BUG: config must serialize");
        let accounts: Vec<String> = loaded
            .extracted_accounts()
            .keys()
            .map(ToString::to_string)
            .collect();

        for widget in migrated["scenes"]
            .as_array()
            .expect("BUG: scenes array")
            .iter()
            .flat_map(|scene| scene["widgets"].as_array().expect("BUG: widgets array"))
        {
            let type_id = widget["widget_type_id"]
                .as_str()
                .expect("BUG: widget_type_id must be a string");
            assert_ne!(
                type_id, NIL_UUID,
                "{case}: an upgraded widget kept a nil type id: {widget}",
            );
            let params = &widget["params"];
            assert!(
                params.get("_legacy").is_none() && params.get("_legacy_remote").is_none(),
                "{case}: retired placeholder shape leaked through: {widget}",
            );
            // A binding naming an account that did not survive leaves the
            // widget pointing at nothing, which reads as "never configured".
            for account in widget["credential_bindings"]
                .as_object()
                .into_iter()
                .flat_map(|slots| slots.values())
            {
                let account = account.as_str().expect("BUG: a binding names an account");
                assert!(
                    accounts.iter().any(|id| id == account),
                    "{case}: binding names account {account}, which did not migrate: {accounts:?}",
                );
            }
        }
    }
}

#[tokio::test]
async fn current_version_config_is_a_noop() {
    let tmp = tempdir();
    let dest = tmp.path().join("bmc_config.json");
    fs::write(
        &dest,
        format!(r#"{{"version":{CONFIG_VERSION},"scenes":[],"accounts":[]}}"#),
    )
    .await
    .expect("BUG: seed write should succeed");

    let loaded = migrate_and_persist(&dest)
        .await
        .expect("BUG: no-op migration should succeed");

    assert!(
        !loaded.was_migrated(),
        "already-versioned config must be a no-op"
    );
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

#[test]
fn v1_config_reshapes_account_to_typed_credential() {
    // A v1 (pre-typed-credential) config migrates to the current schema:
    // the closed braiins_pool + api_key account becomes a typed
    // braiins-pool credential instance with a `token` field value.
    let v1 = r#"{
        "version": 1,
        "scenes": [],
        "accounts": [{
            "id": "11111111-1111-1111-1111-111111111111",
            "type": "braiins_pool",
            "name": "primary",
            "authentication": { "api_key": "sk-secret" },
            "created_at": "2025-01-01T00:00:00Z"
        }]
    }"#;

    let loaded = LoadedConfig::from_str(v1).expect("BUG: v1 config must parse");
    assert!(loaded.was_migrated(), "a v1 config must be migrated");
    assert!(
        loaded.report().is_none(),
        "the v1 hop carries no widget report"
    );

    assert_eq!(loaded.current().version, CONFIG_VERSION);
    let account = loaded
        .extracted_accounts()
        .values()
        .next()
        .expect("BUG: the account must survive the reshape");
    assert_eq!(account.type_id, "braiins-pool");
    assert_eq!(account.field_values["token"], "sk-secret");
}

#[tokio::test]
async fn unknown_future_version_is_rejected() {
    let tmp = tempdir();
    let dest = tmp.path().join("bmc_config.json");
    fs::write(&dest, r#"{"version":999,"scenes":[]}"#)
        .await
        .expect("BUG: seed write should succeed");

    let err = migrate_and_persist(&dest)
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
async fn invalid_migration_leaves_the_original_on_disk() {
    // A migration that produces an invalid config must not overwrite
    // the readable original. Two widgets sharing a position in a
    // combined scene overlap, so the migrated config fails validation;
    // the load→validate→persist chain must reject it *before* writing
    // and leave the v0 file intact (BDK-346).
    let tmp = tempdir();
    let dest = tmp.path().join("bmc_config.json");
    let original = r#"{
        "scenes": [
            {
                "id": "a418d38d-a506-489d-9627-0c7909374ef1",
                "enabled": true,
                "kind": "combined",
                "widgets": [
                    {"id":"3c32f8c7-e678-466d-a331-39b5c8f89153","row":0,"col":0,"size":"medium","kind":"clock","params":{}},
                    {"id":"8521767f-5659-4b2a-8790-75d5b2f154cf","row":0,"col":0,"size":"medium","kind":"clock","params":{}}
                ]
            }
        ],
        "accounts": []
    }"#;
    fs::write(&dest, original)
        .await
        .expect("BUG: seed write should succeed");

    let err = migrate_and_persist(&dest)
        .await
        .expect_err("an invalid migration must fail rather than persist");
    assert!(
        format!("{err:#}").contains("failed validation"),
        "error should name the validation failure (got: {err:#})"
    );

    // The original v0 file must still be on disk, byte-for-byte, with
    // no backup written (nothing was persisted).
    let on_disk = fs::read_to_string(&dest)
        .await
        .expect("BUG: original file should still be readable");
    assert_eq!(on_disk, original, "the original config must be untouched");

    let parent = dest.parent().expect("BUG: tmp path has a parent");
    let mut entries = fs::read_dir(parent).await.expect("BUG: readdir");
    while let Ok(Some(entry)) = entries.next_entry().await {
        assert!(
            !entry.file_name().to_string_lossy().contains(".backup."),
            "a rejected migration must not write a backup",
        );
    }
}

#[tokio::test]
async fn load_is_pure_without_persist() {
    // `LoadedConfig::from_str` is the pure, in-memory path. No
    // filesystem side effects; the caller is free to inspect the
    // result and discard it.
    let loaded =
        LoadedConfig::from_str(FIXTURE).expect("BUG: fixture must parse via the pure FromStr path");
    assert!(loaded.was_migrated());
    assert_eq!(
        loaded.current().version,
        CONFIG_VERSION,
        "FromStr path must upgrade to the current schema"
    );
}

#[tokio::test]
async fn legacy_path_is_relocated_on_first_load() {
    // On upgrade, the config used to live at
    // `<parent>/bmc_config.json` and is now expected at
    // `<parent>/bmc/config.json` so it can be a conffile under a
    // preserved directory. `load_any_version` triggers the copy
    // automatically when it sees the new path missing but the
    // legacy sibling present. The legacy file stays around so a
    // forced boot into old firmware still finds its config.
    let tmp = tempdir();
    let new_dir = tmp.path().join("bmc");
    let new_path = new_dir.join("config.json");
    let legacy_path = tmp.path().join("bmc_config.json");

    // Seed the legacy path, current schema body.
    fs::write(
        &legacy_path,
        format!(r#"{{"version":{CONFIG_VERSION},"scenes":[],"accounts":[]}}"#),
    )
    .await
    .expect("BUG: seed legacy file");

    let loaded = config_migration::load_any_version(&new_path)
        .await
        .expect("BUG: load must succeed after relocation");

    assert!(!loaded.was_migrated(), "current-version file is a no-op");
    assert!(
        fs::try_exists(&new_path).await.unwrap_or(false),
        "new path must exist after load_any_version"
    );
    assert!(
        fs::try_exists(&legacy_path).await.unwrap_or(false),
        "legacy path must remain intact after the copy — downgrade safety"
    );
}

#[tokio::test]
async fn legacy_path_ignored_when_new_path_already_exists() {
    // If both the new and legacy paths exist, the new path wins and
    // the legacy file is left alone — relocation must never overwrite
    // a real config.
    let tmp = tempdir();
    let new_dir = tmp.path().join("bmc");
    fs::create_dir_all(&new_dir)
        .await
        .expect("BUG: mkdir new dir");
    let new_path = new_dir.join("config.json");
    let legacy_path = tmp.path().join("bmc_config.json");

    fs::write(
        &new_path,
        format!(r#"{{"version":{CONFIG_VERSION},"scenes":[],"accounts":[]}}"#),
    )
    .await
    .expect("BUG: seed new file");
    fs::write(&legacy_path, r#"{"ignored":true}"#)
        .await
        .expect("BUG: seed legacy file");

    config_migration::load_any_version(&new_path)
        .await
        .expect("BUG: load must succeed");

    assert!(
        fs::try_exists(&legacy_path).await.unwrap_or(false),
        "legacy path must remain untouched when new exists"
    );
}

/// CLI smoke: the offline `bmc-migrate-config <src> <dst>` tool exits
/// 0, writes the upgraded config to `<dst>`, and prints a counts
/// report. Exercises the shipped binary end to end so a QA/CI run can
/// migrate a captured sample without flashing firmware.
#[tokio::test]
async fn cli_smoke_migrates_fixture_and_reports_counts() {
    let tmp = tempdir();
    let src = tmp.path().join("src.json");
    let dst = tmp.path().join("dst.json");
    fs::write(&src, FIXTURE)
        .await
        .expect("BUG: seed fixture write should succeed");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_bmc-migrate-config"))
        .arg(&src)
        .arg(&dst)
        .output()
        .expect("BUG: bmc-migrate-config must run");

    assert!(
        output.status.success(),
        "CLI must exit 0 (stderr: {})",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("BUG: stdout must be UTF-8");
    assert!(
        stdout.contains("was_migrated=true") && stdout.contains("translated_widgets="),
        "CLI must emit a counts report (got: {stdout})"
    );
    assert!(
        fs::try_exists(&dst).await.unwrap_or(false),
        "CLI must write the upgraded config to <dst>"
    );
}

/// The offline tool must refuse to write a config the device would
/// reject and wipe on next boot: it validates the (migrated) config
/// with the same rules as the boot path before persisting, exits
/// non-zero on failure, and leaves `<dst>` untouched. Regression guard
/// for the gap where the CLI could bless an invalid config the device
/// then throws away.
#[tokio::test]
async fn cli_refuses_to_write_a_config_that_would_fail_validation() {
    let tmp = tempdir();

    // Start from a valid current-schema config (migrate the fixture in
    // place), then tamper one field so it fails `validate` while still
    // parsing as the current schema.
    let valid = tmp.path().join("valid.json");
    fs::write(&valid, FIXTURE)
        .await
        .expect("BUG: seed fixture write should succeed");
    migrate_and_persist(&valid)
        .await
        .expect("BUG: fixture must migrate to a valid current config");

    let raw = fs::read_to_string(&valid)
        .await
        .expect("BUG: migrated config must be readable");
    let mut value: serde_json::Value =
        serde_json::from_str(&raw).expect("BUG: migrated config must be valid JSON");
    // A global cycle duration below the 1s minimum is rejected by
    // `validate_scene_cycling` yet deserializes fine as the current
    // schema.
    value["scene_cycling"] = serde_json::json!({
        "automatic_cycling_enabled": true,
        "automatic_cycling_default_duration": "0s",
        "transition": "slide",
    });

    let src = tmp.path().join("invalid_src.json");
    let dst = tmp.path().join("invalid_dst.json");
    fs::write(
        &src,
        serde_json::to_vec(&value).expect("BUG: reserialize must succeed"),
    )
    .await
    .expect("BUG: invalid config write should succeed");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_bmc-migrate-config"))
        .arg(&src)
        .arg(&dst)
        .output()
        .expect("BUG: bmc-migrate-config must run");

    assert!(
        !output.status.success(),
        "CLI must exit non-zero on a config that fails validation"
    );
    assert!(
        !fs::try_exists(&dst).await.unwrap_or(false),
        "CLI must not write <dst> when validation fails"
    );
}

/// Unique temp dir for each test, auto-removed when the returned
/// `TempDir` drops at the end of the test.
fn tempdir() -> TempDir {
    tempfile::tempdir().expect("BUG: tmp dir creation should succeed")
}
