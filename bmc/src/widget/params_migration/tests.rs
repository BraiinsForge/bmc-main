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

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use bmc_widget_manifest::{Manifest, ParamKey, ParamValue, ProgramEntry};
use indoc::{formatdoc, indoc};
use serde_json::{Value, json};
use tokio::sync::RwLock;

use super::{
    MIGRATOR_STDOUT_LIMIT, MigrationFailure, MigrationJob, Migrator, MigratorStatus,
    MigratorViewport, SUPPORTED_MIGRATOR_PROTOCOL, commit_migrated_params, merge_overlay,
    migrator_status, run_migrator,
};
use crate::config::ConfigHandle;
use crate::scene::{Scene, Widget, WidgetPlacement, WidgetPosition};
use crate::test_support::manifest::{param_key, string_list_param, string_param, widget_manifest};
use crate::test_support::{answering_migrator, migrator_script};
use crate::widget::WidgetInfo;
use crate::widget::registry::ParamsMigrator;

fn manifest() -> Manifest {
    let mut manifest = widget_manifest(
        uuid::Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0000),
        semver::Version::new(0, 2, 0),
        317,
        238,
    );
    manifest
        .params
        .insert(param_key("symbols"), string_list_param(&["NVDA"]));
    manifest
        .params
        .insert(param_key("period"), string_param("7d"));
    manifest.programs.params_migrator = Some(ProgramEntry {
        path: PathBuf::from("migrate"),
        protocol: SUPPORTED_MIGRATOR_PROTOCOL,
    });
    manifest
}

fn old_params() -> BTreeMap<ParamKey, ParamValue> {
    [
        (
            param_key("symbol_1"),
            ParamValue::String("EUR-USD".to_owned()),
        ),
        (param_key("symbol_2"), ParamValue::String(String::new())),
        (param_key("period"), ParamValue::String("1d".to_owned())),
    ]
    .into_iter()
    .collect()
}

fn write_migrator(dir: &std::path::Path, body: &str) -> PathBuf {
    let path = dir.join("migrate");
    crate::test_support::write_executable(&path, body);
    path
}

fn info_with(dir: &std::path::Path, migrator: Option<PathBuf>, manifest: Manifest) -> WidgetInfo {
    let mut info = WidgetInfo::for_test(manifest, dir.to_path_buf(), dir.join("widget"), None);
    let protocol = info
        .manifest
        .programs
        .params_migrator
        .as_ref()
        .map_or(SUPPORTED_MIGRATOR_PROTOCOL, |program| program.protocol);
    info.params_migrator = migrator.map(|path| ParamsMigrator { path, protocol });
    info
}

const VIEWPORT: MigratorViewport = MigratorViewport {
    width: 317,
    height: 238,
    viewport_shape: bmc_widget_protocol::ViewportShape::Rectangular,
    display: bmc_widget_protocol::DisplayInfo::BMC100,
};

const JOB_WIDGET_ID: uuid::Uuid = uuid::Uuid::from_u128(0x7a22_0000_0000_4000_8000_0000_0000_0001);

async fn run(
    dir: &std::path::Path,
    body: &str,
) -> Result<BTreeMap<ParamKey, ParamValue>, MigrationFailure> {
    run_with(dir, body, old_params()).await
}

async fn run_with(
    dir: &std::path::Path,
    body: &str,
    stored: BTreeMap<ParamKey, ParamValue>,
) -> Result<BTreeMap<ParamKey, ParamValue>, MigrationFailure> {
    run_stamped(dir, body, stored, None).await
}

async fn run_stamped(
    dir: &std::path::Path,
    body: &str,
    stored: BTreeMap<ParamKey, ParamValue>,
    params_version: Option<&semver::Version>,
) -> Result<BTreeMap<ParamKey, ParamValue>, MigrationFailure> {
    let migrator = write_migrator(dir, body);
    let info = info_with(dir, Some(migrator.clone()), manifest());
    let job = MigrationJob {
        scene_id: crate::scene::SceneId::generate(),
        widget_id: crate::scene::WidgetId::from(JOB_WIDGET_ID),
        stored_params: &stored,
        params_version,
        migrator: Migrator {
            installed: &info,
            path: &migrator,
        },
        viewport: VIEWPORT,
    };
    run_migrator(&job).await
}

#[tokio::test]
async fn an_empty_overlay_drops_undeclared_keys_and_fills_defaults() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let params = run(temp.path(), &answering_migrator(&json!({})))
        .await
        .expect("BUG: {} is accepted");
    assert_eq!(
        params,
        [
            (param_key("period"), ParamValue::String("1d".to_owned())),
            (
                param_key("symbols"),
                ParamValue::List(vec![ParamValue::String("NVDA".to_owned())])
            ),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>()
    );
}

#[tokio::test]
async fn an_overlay_rewrites_the_declared_keys() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let params = run(
        temp.path(),
        &answering_migrator(&json!({"symbols": ["EUR-USD", "BTC-USD"]})),
    )
    .await
    .expect("BUG: a valid overlay is accepted");
    assert_eq!(
        params[&param_key("symbols")],
        ParamValue::List(vec![
            ParamValue::String("EUR-USD".to_owned()),
            ParamValue::String("BTC-USD".to_owned()),
        ])
    );
    assert_eq!(
        params[&param_key("period")],
        ParamValue::String("1d".to_owned())
    );
    assert!(!params.contains_key(&param_key("symbol_1")));
}

#[tokio::test]
async fn the_program_sees_the_stamp_the_versions_the_params_and_the_geometry() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let body = migrator_script("cat > stdin.json", &json!({}));
    run(temp.path(), &body)
        .await
        .expect("BUG: dumping program succeeds");

    let input: Value = serde_json::from_slice(
        &std::fs::read(temp.path().join("stdin.json")).expect("BUG: stdin dumped"),
    )
    .expect("BUG: json");
    assert_eq!(input["params_version"], Value::Null);
    assert_eq!(input["widget_version"], "0.2.0");
    assert_eq!(input["params"]["symbol_1"], "EUR-USD");
    assert_eq!(input["width"], 317);
    assert_eq!(input["height"], 238);
    assert_eq!(input["viewport_shape"], "rectangular");
    assert_eq!(
        input["display"],
        json!({"width": 1_280, "height": 480, "shape": "rectangular", "dpi": 217})
    );
    let keys: std::collections::BTreeSet<&str> = input
        .as_object()
        .expect("BUG: stdin is an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        std::collections::BTreeSet::from([
            "display",
            "height",
            "params",
            "params_version",
            "viewport_shape",
            "widget_version",
            "width",
        ]),
        "protocol 0 has a fixed key set, with no settings and no credentials"
    );
}

#[tokio::test]
async fn stderr_past_the_cap_is_drained_not_fatal() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let body = migrator_script(
        &format!("yes e | head -c {} >&2", super::MIGRATOR_STDERR_LIMIT * 4),
        &json!({}),
    );
    run(temp.path(), &body)
        .await
        .expect("BUG: a chatty program still finishes and its {} is accepted");
}

#[tokio::test]
async fn a_program_filling_stderr_before_reading_a_large_input_still_finishes() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let mut stored = old_params();
    stored.insert(
        param_key("symbol_1"),
        ParamValue::String("p".repeat(80_000)),
    );
    let params = run_with(
        temp.path(),
        &migrator_script(
            indoc! {"
                yes e | head -c 70000 >&2
                cat > /dev/null
            "},
            &json!({}),
        ),
        stored,
    )
    .await
    .expect("BUG: both pipes past their buffers must not wedge the exchange");
    assert_eq!(
        params[&param_key("period")],
        ParamValue::String("1d".to_owned())
    );
}

#[tokio::test]
async fn a_program_that_ignores_a_large_input_still_answers() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let mut stored = old_params();
    stored.insert(
        param_key("symbol_1"),
        ParamValue::String("p".repeat(80_000)),
    );
    let params = run_with(
        temp.path(),
        &migrator_script("exec <&-", &json!({})),
        stored,
    )
    .await
    .expect("BUG: a closed stdin is the program's business, not a failure");
    assert_eq!(
        params[&param_key("period")],
        ParamValue::String("1d".to_owned())
    );
}

#[tokio::test]
async fn the_program_runs_in_the_widget_dir() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    run(temp.path(), &migrator_script("pwd > cwd", &json!({})))
        .await
        .expect("BUG: program succeeds");
    let cwd = std::fs::read_to_string(temp.path().join("cwd")).expect("BUG: cwd dumped");
    assert_eq!(
        std::fs::canonicalize(cwd.trim()).expect("BUG: cwd"),
        std::fs::canonicalize(temp.path()).expect("BUG: tempdir")
    );
}

#[tokio::test]
async fn a_non_zero_exit_is_a_failure() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let script = indoc! {"
        #!/bin/sh
        echo 'boom' >&2
        exit 3
    "};
    let result = run(temp.path(), script).await;
    assert!(
        matches!(result, Err(MigrationFailure::Exit(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn each_stderr_line_reaches_the_log() {
    use tracing_subscriber::layer::SubscriberExt as _;

    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let logs = crate::test_support::LogCapture::default();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(logs.clone()),
    );
    let _guard = tracing::subscriber::set_default(subscriber);
    let script = migrator_script(
        indoc! {"
            echo 'first' >&2
            echo 'second' >&2
        "},
        &json!({}),
    );
    // tracing caches a callsite's interest process-wide on first reach,
    // so a sibling test reaching it from a subscriber-less thread would mute it:
    // reach it once, rebuild the cache under this subscriber, then judge the second run.
    run(temp.path(), &script)
        .await
        .expect("BUG: an empty overlay over the stored params is valid");
    tracing::callsite::rebuild_interest_cache();

    run(temp.path(), &script)
        .await
        .expect("BUG: an empty overlay over the stored params is valid");

    let text = logs.text();
    assert!(
        text.contains("migrator: first") && text.contains("migrator: second"),
        "the log is where a migrator author sees what the program said: {text}"
    );
}

#[tokio::test]
async fn garbage_on_stdout_is_a_failure() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let result = run(
        temp.path(),
        indoc! {"
            #!/bin/sh
            echo 'not json'
        "},
    )
    .await;
    assert!(
        matches!(result, Err(MigrationFailure::NotAnObject)),
        "{result:?}"
    );
    let result = run(temp.path(), &answering_migrator(&json!([1]))).await;
    assert!(
        matches!(result, Err(MigrationFailure::NotAnObject)),
        "{result:?}"
    );
}

#[tokio::test]
async fn stdout_over_the_cap_is_a_failure() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let body = formatdoc! {"
        #!/bin/sh
        yes x | head -c {}
    ", MIGRATOR_STDOUT_LIMIT + 1};
    let result = run(temp.path(), &body).await;
    assert!(
        matches!(result, Err(MigrationFailure::StdoutTooLarge)),
        "{result:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_hung_program_times_out() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let handle = tokio::spawn({
        let dir = temp.path().to_path_buf();
        async move {
            run(
                &dir,
                indoc! {"
                    #!/bin/sh
                    exec sleep 1000
                "},
            )
            .await
        }
    });
    tokio::task::yield_now().await;
    tokio::time::advance(super::MIGRATOR_TIMEOUT + std::time::Duration::from_secs(1)).await;
    let result = handle.await.expect("BUG: task joins");
    assert!(
        matches!(result, Err(MigrationFailure::Timeout)),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_rejected_overlay_is_a_failure() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let result = run(temp.path(), &answering_migrator(&json!({"symbols": []}))).await;
    assert!(
        matches!(result, Err(MigrationFailure::Rejected(_))),
        "min_items 1: {result:?}"
    );
    let result = run(temp.path(), &answering_migrator(&json!({"unknown": 1}))).await;
    assert!(
        matches!(result, Err(MigrationFailure::Rejected(_))),
        "unknown key: {result:?}"
    );
}

#[test]
fn merge_keeps_declared_stored_keys_under_the_overlay() {
    let manifest = manifest();
    let overlay = json!({"symbols": ["A"]});
    let overlay = overlay.as_object().expect("BUG: an object literal");
    let merged = merge_overlay(&manifest, &old_params(), overlay).expect("BUG: valid");
    assert_eq!(merged.len(), 2);
    assert_eq!(
        merged[&param_key("period")],
        ParamValue::String("1d".to_owned())
    );
}

#[test]
fn an_unsupported_protocol_is_not_usable() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let mut protocol_1 = manifest();
    protocol_1
        .programs
        .params_migrator
        .as_mut()
        .expect("BUG: the test package ships a migrator")
        .protocol = 1;
    let info = info_with(temp.path(), Some(temp.path().join("migrate")), protocol_1);
    assert!(
        matches!(
            migrator_status(&info),
            MigratorStatus::UnsupportedProtocol(1)
        ),
        "{:?}",
        migrator_status(&info)
    );
    let info = info_with(temp.path(), Some(temp.path().join("migrate")), manifest());
    assert!(
        matches!(migrator_status(&info), MigratorStatus::Usable(migrator) if migrator.path == temp.path().join("migrate")),
        "{:?}",
        migrator_status(&info)
    );
}

async fn config_with_widget(
    dir: &std::path::Path,
    widget: Widget,
) -> (Arc<RwLock<ConfigHandle>>, crate::scene::SceneId) {
    let (mut config, _accounts) = ConfigHandle::init(
        dir.join("settings.json"),
        50,
        50,
        50,
        50,
        bmc_platform::Product::Bmc100,
    )
    .await;
    let mut scene = Scene::combined();
    scene.widgets.insert(widget.id, widget);
    config.scenes_mut().clear();
    config.scenes_mut().insert(scene.id, scene.clone());
    (Arc::new(RwLock::new(config)), scene.id)
}

fn widget_with(params: BTreeMap<ParamKey, ParamValue>, stamp: Option<semver::Version>) -> Widget {
    let mut widget = Widget::new(
        uuid::Uuid::new_v4(),
        params,
        WidgetPosition { row: 0, col: 0 },
        WidgetPlacement::SlotSpan(crate::scene::SlotSpan {
            columns: 1,
            rows: 1,
        }),
    );
    widget.params_version = stamp;
    widget
}

/// Commit `params` the way a job that read `seen` would, against the `manifest()` package at 0.2.0.
async fn commit_as_seen(
    dir: &std::path::Path,
    config: &Arc<RwLock<ConfigHandle>>,
    scene_id: crate::scene::SceneId,
    seen: &Widget,
    params: BTreeMap<ParamKey, ParamValue>,
) -> Vec<crate::scene::WidgetId> {
    let migrator = dir.join("migrate");
    let info = info_with(dir, Some(migrator.clone()), manifest());
    let job = MigrationJob {
        scene_id,
        widget_id: seen.id,
        stored_params: &seen.params,
        params_version: seen.params_version.as_ref(),
        migrator: Migrator {
            installed: &info,
            path: &migrator,
        },
        viewport: VIEWPORT,
    };
    commit_migrated_params(config, vec![(job, params)]).await
}

#[tokio::test]
async fn commit_stamps_and_saves_when_the_stamp_is_unmoved() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let widget = widget_with(old_params(), None);
    let (config, scene_id) = config_with_widget(temp.path(), widget.clone()).await;
    let new_params: BTreeMap<_, _> = [(param_key("period"), ParamValue::String("1h".to_owned()))]
        .into_iter()
        .collect();

    let committed =
        commit_as_seen(temp.path(), &config, scene_id, &widget, new_params.clone()).await;

    assert_eq!(committed, vec![widget.id]);
    let stored = config.read().await.scenes()[&scene_id].widgets[&widget.id].clone();
    assert_eq!(stored.params, new_params);
    assert_eq!(stored.params_version, Some(semver::Version::new(0, 2, 0)));
    let saved: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(temp.path().join("settings.json")).expect("BUG: config saved"),
    )
    .expect("BUG: saved config is JSON");
    let saved_widget = &saved["scenes"][0]["widgets"][0];
    assert_eq!(saved_widget["id"], json!(widget.id));
    assert_eq!(saved_widget["params"]["period"], "1h");
    assert_eq!(saved_widget["params_version"], "0.2.0");
}

#[tokio::test]
async fn commit_keeps_the_migrated_params_in_memory_when_the_save_fails() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let widget = widget_with(old_params(), None);
    let (config, scene_id) = config_with_widget(temp.path(), widget.clone()).await;
    let config_path = temp.path().join("settings.json");
    std::fs::remove_file(&config_path).ok();
    std::fs::create_dir(&config_path).expect("BUG: block the config path");
    let new_params: BTreeMap<_, _> = [(param_key("period"), ParamValue::String("1h".to_owned()))]
        .into_iter()
        .collect();

    let committed =
        commit_as_seen(temp.path(), &config, scene_id, &widget, new_params.clone()).await;

    assert_eq!(committed, vec![widget.id]);
    let stored = config.read().await.scenes()[&scene_id].widgets[&widget.id].clone();
    assert_eq!(
        stored.params, new_params,
        "a failed write keeps the change in memory, like every config save"
    );
    assert_eq!(stored.params_version, Some(semver::Version::new(0, 2, 0)));
}

#[tokio::test]
async fn commit_discards_the_result_when_the_stamp_moved() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let seen = widget_with(old_params(), None);
    let mut saved_meanwhile = seen.clone();
    saved_meanwhile.params_version = Some(semver::Version::new(0, 2, 0));
    let (config, scene_id) = config_with_widget(temp.path(), saved_meanwhile).await;
    let new_params: BTreeMap<_, _> = [(param_key("period"), ParamValue::String("1h".to_owned()))]
        .into_iter()
        .collect();

    let committed = commit_as_seen(temp.path(), &config, scene_id, &seen, new_params).await;

    assert!(committed.is_empty(), "{committed:?}");
    let stored = config.read().await.scenes()[&scene_id].widgets[&seen.id].clone();
    assert_eq!(stored.params, old_params(), "a UI save during the run wins");
}

#[tokio::test]
async fn commit_discards_the_result_when_the_widget_was_removed() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let seen = widget_with(old_params(), None);
    let (config, scene_id) = config_with_widget(temp.path(), seen.clone()).await;
    config
        .write()
        .await
        .scenes_mut()
        .get_mut(&scene_id)
        .expect("BUG: the scene was just added")
        .widgets
        .shift_remove(&seen.id);
    let new_params: BTreeMap<_, _> = [(param_key("period"), ParamValue::String("1h".to_owned()))]
        .into_iter()
        .collect();

    let committed = commit_as_seen(temp.path(), &config, scene_id, &seen, new_params).await;

    assert!(committed.is_empty(), "{committed:?}");
    assert!(
        config.read().await.scenes()[&scene_id].widgets.is_empty(),
        "a widget the user removed must not come back with migrated params"
    );
}

#[tokio::test]
async fn commit_takes_the_results_after_a_discarded_one() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let removed = widget_with(old_params(), None);
    let moved = widget_with(old_params(), None);
    let unmoved = widget_with(old_params(), None);
    let mut moved_meanwhile = moved.clone();
    moved_meanwhile.params_version = Some(semver::Version::new(0, 2, 0));
    let (config, scene_id) = config_with_widget(temp.path(), moved_meanwhile).await;
    config
        .write()
        .await
        .scenes_mut()
        .get_mut(&scene_id)
        .expect("BUG: the scene was just added")
        .widgets
        .insert(unmoved.id, unmoved.clone());
    let migrator = temp.path().join("migrate");
    let info = info_with(temp.path(), Some(migrator.clone()), manifest());
    let new_params: BTreeMap<_, _> = [(param_key("period"), ParamValue::String("1h".to_owned()))]
        .into_iter()
        .collect();
    let results = [&removed, &moved, &unmoved].map(|seen| {
        let job = MigrationJob {
            scene_id,
            widget_id: seen.id,
            stored_params: &seen.params,
            params_version: seen.params_version.as_ref(),
            migrator: Migrator {
                installed: &info,
                path: &migrator,
            },
            viewport: VIEWPORT,
        };
        (job, new_params.clone())
    });

    let committed = commit_migrated_params(&config, Vec::from(results)).await;

    assert_eq!(
        committed,
        vec![unmoved.id],
        "a discarded result must not end the batch before the results after it"
    );
}

#[tokio::test]
async fn a_discarded_result_announces_no_scene_change() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let seen = widget_with(old_params(), None);
    let mut saved_meanwhile = seen.clone();
    saved_meanwhile.params_version = Some(semver::Version::new(0, 2, 0));
    let (config, scene_id) = config_with_widget(temp.path(), saved_meanwhile).await;
    config
        .write()
        .await
        .save()
        .await
        .expect("BUG: settle the fixture's own scene edit");
    let mut scene_changes = config.read().await.subscribe_scenes_change();
    let new_params: BTreeMap<_, _> = [(param_key("period"), ParamValue::String("1h".to_owned()))]
        .into_iter()
        .collect();

    let committed = commit_as_seen(temp.path(), &config, scene_id, &seen, new_params).await;
    config
        .write()
        .await
        .save()
        .await
        .expect("BUG: an unrelated save");

    assert!(committed.is_empty(), "{committed:?}");
    assert!(
        scene_changes.try_recv().is_err(),
        "a discarded result changed no scene, so the next save must not re-push credentials"
    );
}

#[tokio::test]
async fn a_stamped_widget_tells_the_program_its_version() {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let body = migrator_script("cat > stdin.json", &json!({}));
    let stamp = semver::Version::new(0, 1, 0);
    run_stamped(temp.path(), &body, old_params(), Some(&stamp))
        .await
        .expect("BUG: dumping program succeeds");

    let input: Value = serde_json::from_slice(
        &std::fs::read(temp.path().join("stdin.json")).expect("BUG: stdin dumped"),
    )
    .expect("BUG: json");
    assert_eq!(
        input["params_version"], "0.1.0",
        "the program must learn which shape a stamped widget holds"
    );
}
