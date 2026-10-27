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

//! The params migration gates: boot and package reload.

use std::path::PathBuf;

use bmc_widget_manifest::ProgramEntry;
use indoc::{formatdoc, indoc};
use serde_json::{Value, json};
use tracing_subscriber::prelude::*;

use super::*;
use crate::test_support::manifest::{param_key, string_list_param, string_param, write_manifest};
use crate::test_support::{SLEEPING_WIDGET, answering_migrator, migrator_script};
use crate::widget::params_migration::SUPPORTED_MIGRATOR_PROTOCOL;

fn manifest_with_symbols(version: &str) -> Manifest {
    let mut manifest = package_manifest(version);
    manifest
        .params
        .insert(param_key("symbols"), string_list_param(&["NVDA"]));
    manifest
}

fn migrating_manifest(version: &str, protocol: u32) -> Manifest {
    let mut manifest = manifest_with_symbols(version);
    manifest.programs.params_migrator = Some(ProgramEntry {
        path: PathBuf::from("migrate"),
        protocol,
    });
    manifest
}

fn install_manifest(temp: &tempfile::TempDir, manifest: &Manifest) {
    write_manifest(&temp.path().join("widget-package"), manifest);
}

/// Holds a migrator until the test writes `release` into the widget dir.
const UNTIL_RELEASED: &str = "until [ -e release ]; do sleep 0.05; done";

fn install_migrator(temp: &tempfile::TempDir, version: &str, body: &str) {
    install_migrator_speaking(temp, version, SUPPORTED_MIGRATOR_PROTOCOL, body);
}

fn install_migrator_speaking(temp: &tempfile::TempDir, version: &str, protocol: u32, body: &str) {
    install_manifest(temp, &migrating_manifest(version, protocol));
    crate::test_support::write_executable(
        &temp.path().join("widget-package").join("migrate"),
        body,
    );
}

fn install_package_without_migrator(temp: &tempfile::TempDir, version: &str) {
    install_manifest(temp, &manifest_with_symbols(version));
}

fn params_from(json: &Value) -> BTreeMap<ParamKey, ParamValue> {
    let Ok(ParamValue::Object(params)) = ParamValue::try_from(json) else {
        panic!("BUG: {json} is not a params object");
    };
    params
}

fn with_params(scene: &mut Scene, json: &Value) -> BTreeMap<ParamKey, ParamValue> {
    let params = params_from(json);
    for widget in scene.widgets.values_mut() {
        widget.params = params.clone();
    }
    params
}

fn stored_params(
    config: &ConfigHandle,
    scene_id: SceneId,
) -> (BTreeMap<ParamKey, ParamValue>, Option<semver::Version>) {
    let widget = config.scenes()[&scene_id]
        .widgets
        .values()
        .next()
        .expect("BUG: one widget");
    (widget.params.clone(), widget.params_version.clone())
}

fn settings_bytes(temp: &tempfile::TempDir) -> Option<Vec<u8>> {
    std::fs::read(temp.path().join("settings.json")).ok()
}

async fn boot(coordinator: &Coordinator, config: &Arc<RwLock<ConfigHandle>>) {
    coordinator
        .spawn_initial_widgets(
            config,
            &LocalizationConfig::default(),
            &Timezone::default(),
            false,
            None,
        )
        .await;
}

fn registered_params(
    compositor: &crate::compositor::testing::RecordingCompositor,
    scene: &Scene,
) -> serde_json::Map<String, Value> {
    let widget_id = scene.widgets.keys().next().expect("BUG: one widget");
    compositor
        .retained_params(WidgetInstanceKey::new(widget_id.as_uuid()))
        .expect("BUG: one registration")
}

#[tokio::test]
async fn boot_migrates_stale_params_before_spawning() {
    let (temp, coordinator, compositor, mut scene) =
        coordinator_with_widget(Some(SLEEPING_WIDGET)).await;
    install_migrator(
        &temp,
        "2.0.0",
        &answering_migrator(&json!({"symbols": ["EUR-USD"]})),
    );
    coordinator.refresh_widgets().await.expect("BUG: refresh");
    with_params(&mut scene, &json!({"symbol_1": "EUR-USD"}));
    let config = config_with_scene(&temp, scene.clone()).await;

    boot(&coordinator, &config).await;

    let (params, stamp) = stored_params(&*config.read().await, scene.id);
    assert_eq!(stamp, Some(semver::Version::new(2, 0, 0)));
    let old_key = param_key("symbol_1");
    assert!(!params.contains_key(&old_key));
    let registered = registered_params(&compositor, &scene);
    assert_eq!(
        registered["symbols"],
        json!(["EUR-USD"]),
        "the spawn must read the migrated params"
    );
    coordinator.widget_manager.shutdown().await;
}

#[tokio::test]
async fn boot_migrates_what_the_widget_stored_and_keeps_what_the_overlay_leaves() {
    let (temp, coordinator, _compositor, mut scene) =
        coordinator_with_widget(Some(SLEEPING_WIDGET)).await;
    let mut manifest = migrating_manifest("2.0.0", SUPPORTED_MIGRATOR_PROTOCOL);
    manifest
        .params
        .insert(param_key("period"), string_param("1d"));
    install_manifest(&temp, &manifest);
    crate::test_support::write_executable(
        &temp.path().join("widget-package").join("migrate"),
        &migrator_script("cat > stdin.json", &json!({"symbols": ["AAPL-USD"]})),
    );
    coordinator.refresh_widgets().await.expect("BUG: refresh");
    with_params(&mut scene, &json!({"symbol_1": "AAPL-USD", "period": "7d"}));
    let config = config_with_scene(&temp, scene.clone()).await;

    boot(&coordinator, &config).await;

    let input: Value = serde_json::from_slice(
        &std::fs::read(temp.path().join("widget-package").join("stdin.json"))
            .expect("BUG: the migrator dumped its stdin"),
    )
    .expect("BUG: json");
    assert_eq!(
        input["params"],
        json!({"symbol_1": "AAPL-USD", "period": "7d"}),
        "the migrator must read what the widget stored"
    );
    let (params, _) = stored_params(&*config.read().await, scene.id);
    assert_eq!(
        serde_json::to_value(&params).expect("BUG: params serialize"),
        json!({"period": "7d", "symbols": ["AAPL-USD"]}),
        "the stored period the overlay leaves alone must survive"
    );
    coordinator.widget_manager.shutdown().await;
}

#[tokio::test]
async fn a_failing_migrator_leaves_the_stored_params_and_spawns_the_widget() {
    let (temp, coordinator, compositor, mut scene) =
        coordinator_with_widget(Some(SLEEPING_WIDGET)).await;
    install_migrator(
        &temp,
        "2.0.0",
        indoc! {"
            #!/bin/sh
            exit 1
        "},
    );
    coordinator.refresh_widgets().await.expect("BUG: refresh");
    let old_params = with_params(&mut scene, &json!({"symbol_1": "EUR-USD"}));
    let config = config_with_scene(&temp, scene.clone()).await;

    boot(&coordinator, &config).await;

    let (params, stamp) = stored_params(&*config.read().await, scene.id);
    assert_eq!(
        params, old_params,
        "a failed migration must not touch the stored params"
    );
    assert_eq!(stamp, None, "a failed migration must not stamp");
    let registered = registered_params(&compositor, &scene);
    assert_eq!(
        serde_json::Value::Object(registered),
        serde_json::to_value(&old_params).expect("BUG: params serialize"),
        "the widget spawns on the stored params"
    );
    coordinator.widget_manager.shutdown().await;
}

#[tokio::test]
async fn package_reload_migrates_before_replacing() {
    let (temp, coordinator, compositor, scene) =
        coordinator_with_widget(Some(SLEEPING_WIDGET)).await;
    let config = config_with_scene(&temp, scene.clone()).await;
    coordinator
        .spawn_configured_scene_widgets(&config, scene.id)
        .await;

    install_migrator(&temp, "2.0.0", &answering_migrator(&json!({})));
    coordinator.reload_changed_widgets(&config).await;

    let (params, stamp) = stored_params(&*config.read().await, scene.id);
    assert_eq!(stamp, Some(semver::Version::new(2, 0, 0)));
    assert_eq!(
        serde_json::to_value(&params).expect("BUG: params serialize")["symbols"],
        json!(["NVDA"]),
        "{{}} on an unstamped widget keeps the shape and fills defaults"
    );
    assert!(matches!(
        coordinator.widget_manager.snapshot().await.widgets.as_slice(),
        [managed] if managed.identity.version == semver::Version::new(2, 0, 0)
    ));
    let widget_id = scene.widgets.keys().next().expect("BUG: one widget");
    let registrations = compositor.registered_params(WidgetInstanceKey::new(widget_id.as_uuid()));
    assert_eq!(registrations.len(), 2, "the original and its replacement");
    assert_eq!(
        registrations[1]["symbols"],
        json!(["NVDA"]),
        "the replacement must register the migrated params"
    );
    assert!(
        compositor
            .parameter_pushes
            .lock()
            .expect("BUG: recording compositor lock must not be poisoned")
            .is_empty(),
        "a replaced widget takes the migrated params from its registration, not a push"
    );
    coordinator.widget_manager.shutdown().await;
}

#[tokio::test]
async fn a_package_without_a_migrator_launches_with_defaults_and_leaves_the_config() {
    let (temp, coordinator, compositor, mut scene) =
        coordinator_with_widget(Some(SLEEPING_WIDGET)).await;
    install_package_without_migrator(&temp, "2.0.0");
    coordinator.refresh_widgets().await.expect("BUG: refresh");
    let old_params = with_params(&mut scene, &json!({"symbol_1": "EUR-USD"}));
    let config = config_with_scene(&temp, scene.clone()).await;
    let before = settings_bytes(&temp);

    boot(&coordinator, &config).await;

    assert_eq!(
        Value::Object(registered_params(&compositor, &scene)),
        json!({"symbols": ["NVDA"]}),
        "the launch drops the undeclared key and fills the default"
    );
    assert_eq!(
        stored_params(&*config.read().await, scene.id),
        (old_params, None),
        "defaults are read at launch, never stored or stamped"
    );
    assert_eq!(
        settings_bytes(&temp),
        before,
        "the config must not be written"
    );
    coordinator.widget_manager.shutdown().await;
}

#[tokio::test]
async fn rejected_params_of_a_package_without_a_migrator_launch_as_stored() {
    let (temp, coordinator, compositor, mut scene) =
        coordinator_with_widget(Some(SLEEPING_WIDGET)).await;
    install_package_without_migrator(&temp, "2.0.0");
    coordinator.refresh_widgets().await.expect("BUG: refresh");
    let old_params = with_params(&mut scene, &json!({"symbols": "NVDA"}));
    let config = config_with_scene(&temp, scene.clone()).await;

    boot(&coordinator, &config).await;

    assert_eq!(
        Value::Object(registered_params(&compositor, &scene)),
        json!({"symbols": "NVDA"}),
        "a value of the wrong type must not be replaced by a default nobody chose"
    );
    assert_eq!(
        stored_params(&*config.read().await, scene.id),
        (old_params, None)
    );
    coordinator.widget_manager.shutdown().await;
}

#[tokio::test]
async fn an_unsupported_protocol_skips_every_instance_and_warns_once() {
    let (temp, coordinator, _compositor, mut scene) =
        coordinator_with_widget(Some(SLEEPING_WIDGET)).await;
    install_migrator_speaking(
        &temp,
        "2.0.0",
        SUPPORTED_MIGRATOR_PROTOCOL + 1,
        &answering_migrator(&json!({"symbols": ["EUR-USD"]})),
    );
    coordinator.refresh_widgets().await.expect("BUG: refresh");
    let first = scene
        .widgets
        .values()
        .next()
        .expect("BUG: one widget")
        .clone();
    let mut twin = first.clone();
    twin.id = crate::scene::WidgetId::generate();
    scene.widgets.insert(twin.id, twin);
    let config = config_with_scene(&temp, scene.clone()).await;
    let logs = crate::test_support::LogCapture::default();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(logs.clone()),
    );
    let _guard = tracing::subscriber::set_default(subscriber);

    let committed = coordinator.migrate_stale_params(&config).await;

    assert!(
        committed.is_empty(),
        "nothing is committed for a protocol this build lacks"
    );
    let stored = config.read().await;
    let widgets = &stored.scenes()[&scene.id].widgets;
    assert_eq!(widgets.len(), 2);
    for widget in widgets.values() {
        assert_eq!(widget.params_version, None, "no run, no stamp");
        assert_eq!(widget.params, first.params, "params stay as stored");
    }
    assert_eq!(
        logs.text().matches("does not implement").count(),
        1,
        "one warning per widget type, not per instance:\n{}",
        logs.text()
    );
}

#[tokio::test]
async fn reload_pushes_migrated_params_to_an_unchanged_widget() {
    let (temp, coordinator, compositor, mut scene) =
        coordinator_with_widget(Some(SLEEPING_WIDGET)).await;
    install_migrator(
        &temp,
        "2.0.0",
        &answering_migrator(&json!({"symbols": ["EUR-USD"]})),
    );
    coordinator.refresh_widgets().await.expect("BUG: refresh");
    for widget in scene.widgets.values_mut() {
        widget.params_version = Some(semver::Version::new(1, 0, 0));
    }
    let config = config_with_scene(&temp, scene.clone()).await;
    coordinator
        .spawn_configured_scene_widgets(&config, scene.id)
        .await;

    coordinator.reload_changed_widgets(&config).await;

    let widget_id = scene.widgets.keys().next().expect("BUG: one widget");
    let key = WidgetInstanceKey::new(widget_id.as_uuid());
    assert_eq!(
        compositor.registered_params(key).len(),
        1,
        "an unchanged package must get a push, not a replace that registers again"
    );
    assert_eq!(
        compositor.retained_params(key).expect("BUG: registered")["symbols"],
        json!(["EUR-USD"]),
        "the running widget must receive the migrated params"
    );
    coordinator.widget_manager.shutdown().await;
}

#[tokio::test]
async fn a_reload_onto_a_package_without_a_migrator_registers_its_defaults() {
    let (temp, coordinator, compositor, scene) =
        coordinator_with_widget(Some(SLEEPING_WIDGET)).await;
    let config = config_with_scene(&temp, scene.clone()).await;
    coordinator
        .spawn_configured_scene_widgets(&config, scene.id)
        .await;
    let before = settings_bytes(&temp);

    install_package_without_migrator(&temp, "2.0.0");
    coordinator.reload_changed_widgets(&config).await;

    let widget_id = scene.widgets.keys().next().expect("BUG: one widget");
    assert_eq!(
        compositor
            .registered_params(WidgetInstanceKey::new(widget_id.as_uuid()))
            .into_iter()
            .map(Value::Object)
            .collect::<Vec<_>>(),
        [json!({"current": 0}), json!({"symbols": ["NVDA"]})],
        "the changed package must register a replacement that reads the stored params through it"
    );
    assert_eq!(
        settings_bytes(&temp),
        before,
        "the config must not be written"
    );
    coordinator.widget_manager.shutdown().await;
}

fn launch_count(launches: &std::path::Path) -> usize {
    std::fs::read_to_string(launches)
        .map(|log| log.lines().count())
        .unwrap_or_default()
}

/// Wait until the widget has launched `count` times and crashed back to a pending restart.
async fn wait_for_pending_restart(
    coordinator: &Coordinator,
    widget_id: crate::scene::WidgetId,
    launches: &std::path::Path,
    count: usize,
    within: Duration,
) {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let snapshot = coordinator.widget_manager.snapshot().await;
        let pending = snapshot.widgets.iter().any(|managed| {
            managed.launch.config_key.instance_id == widget_id.as_uuid()
                && managed.state == ManagedWidgetState::PendingRestart
        });
        if pending && launch_count(launches) >= count {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the widget never crashed back to a pending restart after launch {count}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A crash-looping widget whose params a reload migrated retries promptly,
/// rather than sit out a delay its stale params earned.
#[tokio::test]
async fn reload_retries_a_crash_looping_widget_on_its_migrated_params() {
    /// The respawn delay the default ladder reaches after the reload's own
    /// retry crashes too, so a launch landing inside it is the push's retry.
    const CLIMBED: Duration = Duration::from_secs(4);
    const RETRY_BOUND: Duration = Duration::from_secs(2);

    let (temp, coordinator, _compositor, mut scene) = coordinator_with_widget(None).await;
    let widget_dir = temp.path().join("widget-package");
    let launches = temp.path().join("launches");
    crate::test_support::write_executable(
        &widget_dir.join("widget"),
        &formatdoc! {"
            #!/bin/sh
            echo x >> '{launches}'
            exit 1
        ", launches = launches.display()},
    );
    install_migrator(
        &temp,
        "2.0.0",
        &migrator_script(UNTIL_RELEASED, &json!({"symbols": ["EUR-USD"]})),
    );
    coordinator.refresh_widgets().await.expect("BUG: refresh");
    for widget in scene.widgets.values_mut() {
        widget.params_version = Some(semver::Version::new(1, 0, 0));
    }
    let widget_id = *scene.widgets.keys().next().expect("BUG: one widget");
    let config = config_with_scene(&temp, scene.clone()).await;

    coordinator
        .spawn_configured_scene_widgets(&config, scene.id)
        .await;
    wait_for_pending_restart(&coordinator, widget_id, &launches, 2, 3 * CLIMBED).await;
    let drive = async {
        wait_for_pending_restart(&coordinator, widget_id, &launches, 3, 3 * CLIMBED).await;
        let crashed_at = tokio::time::Instant::now();
        std::fs::write(widget_dir.join("release"), "").expect("BUG: release the migrator");
        crashed_at
    };
    let ((), crashed_at) = tokio::join!(coordinator.reload_changed_widgets(&config), drive);

    let deadline = crashed_at + CLIMBED;
    while launch_count(&launches) < 4 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let elapsed = crashed_at.elapsed();
    assert!(
        launch_count(&launches) >= 4 && elapsed < RETRY_BOUND,
        "the migrated widget must relaunch inside {RETRY_BOUND:?} of its last crash, \
         took {elapsed:?} for {} launches",
        launch_count(&launches)
    );
    coordinator.widget_manager.shutdown().await;
}

/// The cap the docs promise, kept apart from `MIGRATOR_CONCURRENCY`
/// so a change to the constant fails here until the docs follow.
const DOCUMENTED_CAP: usize = 4;

/// How many programs have touched their `started.<pid>` marker in `widget_dir`.
fn started_programs(widget_dir: &std::path::Path) -> usize {
    std::fs::read_dir(widget_dir)
        .expect("BUG: widget dir")
        .map(|entry| entry.expect("BUG: dir entry").file_name())
        .filter(|name| name.to_string_lossy().starts_with("started."))
        .count()
}

#[tokio::test]
async fn a_gate_runs_at_most_four_programs_at_once() {
    let (temp, coordinator, _compositor, mut scene) =
        coordinator_with_widget(Some(SLEEPING_WIDGET)).await;
    install_migrator(
        &temp,
        "2.0.0",
        &migrator_script(
            &formatdoc! {r#"
                touch "started.$$"
                {UNTIL_RELEASED}
            "#},
            &json!({}),
        ),
    );
    coordinator.refresh_widgets().await.expect("BUG: refresh");
    let first = scene
        .widgets
        .values()
        .next()
        .expect("BUG: one widget")
        .clone();
    for _ in 0..DOCUMENTED_CAP {
        let mut twin = first.clone();
        twin.id = crate::scene::WidgetId::generate();
        scene.widgets.insert(twin.id, twin);
    }
    let config = config_with_scene(&temp, scene.clone()).await;
    let widget_dir = temp.path().join("widget-package");
    let observe = async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while started_programs(&widget_dir) < DOCUMENTED_CAP
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let running = started_programs(&widget_dir);
        std::fs::write(widget_dir.join("release"), "").expect("BUG: release the programs");
        running
    };

    let (committed, running) = tokio::join!(coordinator.migrate_stale_params(&config), observe);

    assert_eq!(
        running, DOCUMENTED_CAP,
        "the gate must hold the next program back while the cap's worth run"
    );
    assert_eq!(
        committed.len(),
        DOCUMENTED_CAP + 1,
        "every instance is migrated once the first batch finishes"
    );
}

#[tokio::test]
async fn a_gate_announces_its_migrated_widgets_once() {
    let (temp, coordinator, _compositor, mut scene) =
        coordinator_with_widget(Some(SLEEPING_WIDGET)).await;
    install_migrator(
        &temp,
        "2.0.0",
        &answering_migrator(&json!({"symbols": ["EUR-USD"]})),
    );
    coordinator.refresh_widgets().await.expect("BUG: refresh");
    let mut twin = scene
        .widgets
        .values()
        .next()
        .expect("BUG: one widget")
        .clone();
    twin.id = crate::scene::WidgetId::generate();
    scene.widgets.insert(twin.id, twin);
    let config = config_with_scene(&temp, scene).await;
    config
        .write()
        .await
        .save()
        .await
        .expect("BUG: settle the fixture's own scene edit");
    let mut scene_changes = config.read().await.subscribe_scenes_change();

    let committed = coordinator.migrate_stale_params(&config).await;

    assert_eq!(committed.len(), 2, "both stale instances migrate");
    assert!(scene_changes.try_recv().is_ok(), "the batch is announced");
    assert!(
        scene_changes.try_recv().is_err(),
        "a save per migrated widget would announce each one"
    );
}

fn held_migrator() -> String {
    migrator_script(
        &formatdoc! {"
            touch started
            {UNTIL_RELEASED}
        "},
        &json!({}),
    )
}

/// Try a scene edit once the gate's program has started, then release the
/// program; `true` when the edit had to wait.
async fn edit_waits_while_program_runs(
    coordinator: &Coordinator,
    widget_dir: &std::path::Path,
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !widget_dir.join("started").exists() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let held = tokio::time::timeout(Duration::from_millis(200), coordinator.begin_scene_edit())
        .await
        .is_err();
    std::fs::write(widget_dir.join("release"), "").expect("BUG: release the program");
    held
}

#[tokio::test]
async fn a_reload_holds_scene_edits_until_its_gate_finishes() {
    let (temp, coordinator, _compositor, scene) =
        coordinator_with_widget(Some(SLEEPING_WIDGET)).await;
    let config = config_with_scene(&temp, scene).await;
    install_migrator(&temp, "2.0.0", &held_migrator());
    let widget_dir = temp.path().join("widget-package");

    let ((), held) = tokio::join!(
        coordinator.reload_changed_widgets(&config),
        edit_waits_while_program_runs(&coordinator, &widget_dir)
    );

    assert!(held, "an edit must wait while the reload's gate runs");
    assert!(
        coordinator.begin_scene_edit().await.is_ok(),
        "the reload releases the scenes when it finishes"
    );
    coordinator.widget_manager.shutdown().await;
}

#[tokio::test]
async fn boot_holds_scene_edits_until_its_gate_finishes() {
    let (temp, coordinator, _compositor, scene) =
        coordinator_with_widget(Some(SLEEPING_WIDGET)).await;
    install_migrator(&temp, "2.0.0", &held_migrator());
    coordinator.refresh_widgets().await.expect("BUG: refresh");
    let config = config_with_scene(&temp, scene).await;
    let widget_dir = temp.path().join("widget-package");
    let localization = LocalizationConfig::default();
    let timezone = Timezone::default();

    let ((), held) = tokio::join!(
        coordinator.spawn_initial_widgets(&config, &localization, &timezone, false, None),
        edit_waits_while_program_runs(&coordinator, &widget_dir)
    );

    assert!(held, "an edit must wait while the boot gate runs");
    assert!(
        coordinator.begin_scene_edit().await.is_ok(),
        "boot releases the scenes when it finishes"
    );
    coordinator.widget_manager.shutdown().await;
}

#[tokio::test]
async fn a_stamp_newer_than_the_package_skips_its_migrator_and_keeps_the_config() {
    let (temp, coordinator, compositor, mut scene) =
        coordinator_with_widget(Some(SLEEPING_WIDGET)).await;
    install_migrator(
        &temp,
        "2.0.0",
        &migrator_script("touch ran", &json!({"symbols": ["EUR-USD"]})),
    );
    coordinator.refresh_widgets().await.expect("BUG: refresh");
    let newer_params = with_params(&mut scene, &json!({"symbols": ["AAPL"], "period": "7d"}));
    let newer = semver::Version::new(3, 0, 0);
    for widget in scene.widgets.values_mut() {
        widget.params_version = Some(newer.clone());
    }
    let config = config_with_scene(&temp, scene.clone()).await;
    let before = settings_bytes(&temp);

    boot(&coordinator, &config).await;

    assert!(
        !temp.path().join("widget-package").join("ran").exists(),
        "a migrator only carries params forward, never back"
    );
    assert_eq!(
        Value::Object(registered_params(&compositor, &scene)),
        json!({"symbols": ["AAPL"]}),
        "the older package launches on what it declares"
    );
    assert_eq!(
        stored_params(&*config.read().await, scene.id),
        (newer_params, Some(newer)),
        "the newer package's params and stamp survive the downgrade"
    );
    assert_eq!(
        settings_bytes(&temp),
        before,
        "the config must not be written"
    );
    coordinator.widget_manager.shutdown().await;
}

#[tokio::test]
async fn a_downgrade_and_upgrade_round_trip_keeps_the_newer_params() {
    let (temp, coordinator, compositor, mut scene) =
        coordinator_with_widget(Some(SLEEPING_WIDGET)).await;
    let mut old_manifest = package_manifest("1.0.0");
    old_manifest
        .params
        .insert(param_key("symbol_1"), string_param("NVDA"));
    install_manifest(&temp, &old_manifest);
    coordinator.refresh_widgets().await.expect("BUG: refresh");
    let upgraded = with_params(&mut scene, &json!({"symbols": ["AAPL", "TSLA"]}));
    let version = semver::Version::new(2, 0, 0);
    for widget in scene.widgets.values_mut() {
        widget.params_version = Some(version.clone());
    }
    let config = config_with_scene(&temp, scene.clone()).await;
    boot(&coordinator, &config).await;
    assert_eq!(
        Value::Object(registered_params(&compositor, &scene)),
        json!({"symbol_1": "NVDA"}),
        "the downgraded package launches on its own default"
    );

    install_migrator(
        &temp,
        "2.0.0",
        &migrator_script("touch ran", &json!({"symbols": ["EUR-USD"]})),
    );
    coordinator.reload_changed_widgets(&config).await;

    assert!(
        !temp.path().join("widget-package").join("ran").exists(),
        "params already stamped with the reinstalled version owe no migration"
    );
    assert_eq!(
        Value::Object(registered_params(&compositor, &scene)),
        json!({"symbols": ["AAPL", "TSLA"]}),
        "the upgrade restores the list the downgrade never overwrote"
    );
    assert_eq!(
        stored_params(&*config.read().await, scene.id),
        (upgraded, Some(version))
    );
    coordinator.widget_manager.shutdown().await;
}
