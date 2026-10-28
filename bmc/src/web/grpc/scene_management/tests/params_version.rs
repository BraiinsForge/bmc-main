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

//! The stamp follows the params: set by every add, moved by an update
//! only when the values change.

use std::sync::Arc;

use tokio::sync::RwLock;

use super::{
    RecordingCompositor, add_fullscreen_scene, add_widget, bmc100_caps, empty_secret_store,
    fields_one, string_value, update_widget,
};
use crate::compositor::SlotGrid;
use crate::config::ConfigHandle;
use crate::scene;
use crate::test_support::manifest::{
    exact_viewport, param_key, string_param, widget_manifest, write_manifest,
};
use crate::web::grpc::scene_management::SceneManagementService;
use crate::widget::Coordinator;
use bmc_grpc::web;

const WIDGET_VERSION: semver::Version = semver::Version::new(1, 0, 0);

pub(super) struct Fixture {
    pub(super) service: SceneManagementService,
    pub(super) coordinator: Arc<Coordinator>,
    pub(super) config: Arc<RwLock<ConfigHandle>>,
    widget_uid: uuid::Uuid,
    pub(super) scene_id: scene::SceneId,
}

/// A real package with a sleeping binary, since an added widget is spawned
/// and the coordinator keeps retrying a binary that does not exist.
fn write_stamp_package(package: &std::path::Path) -> uuid::Uuid {
    std::fs::create_dir(package).expect("BUG: create widget package");
    let widget_uid = uuid::Uuid::new_v4();
    let mut manifest = widget_manifest(widget_uid, WIDGET_VERSION, 317, 238);
    manifest.supported_viewports.push(exact_viewport(1280, 480));
    manifest
        .params
        .insert(param_key("label"), string_param("old"));
    write_manifest(package, &manifest);
    crate::test_support::write_executable(
        &package.join("widget"),
        crate::test_support::SLEEPING_WIDGET,
    );
    widget_uid
}

/// Each test ends with `shutdown_widget_manager` to reap the package's widget.
pub(super) async fn fixture() -> (tempfile::TempDir, Fixture) {
    let temp = tempfile::tempdir().expect("BUG: tempdir");
    let widget_uid = write_stamp_package(&temp.path().join("widget-package"));

    let config_path = temp.path().join("bmc-config.json");
    let config = Arc::new(RwLock::new(
        ConfigHandle::init(config_path, 50, 50, 50, 50, bmc_platform::Product::Bmc100)
            .await
            .0,
    ));

    let widget_manager =
        crate::widget::WidgetManager::init(vec![temp.path().to_path_buf()], false).await;
    let registry = widget_manager.registry();
    let compositor: Arc<dyn crate::compositor::Compositor> =
        Arc::new(RecordingCompositor::default());
    let capabilities = bmc100_caps(Some(SlotGrid {
        columns: 4,
        rows: 2,
    }));
    let coordinator = Arc::new(Coordinator::new(
        widget_manager,
        compositor,
        Some("test-display".to_owned()),
        Arc::clone(&registry),
        capabilities,
        empty_secret_store().await,
    ));

    let (led_tx, _led_rx) = tokio::sync::mpsc::channel(16);
    let led_coordinator = crate::led_coordinator::spawn_led_coordinator(led_tx);
    let service = SceneManagementService::new(
        registry,
        Arc::clone(&config),
        empty_secret_store().await,
        Arc::clone(&coordinator),
        capabilities,
        led_coordinator,
    );

    let scene = scene::Scene::combined();
    let scene_id = scene.id;
    config.write().await.scenes_mut().insert(scene_id, scene);
    (
        temp,
        Fixture {
            service,
            coordinator,
            config,
            widget_uid,
            scene_id,
        },
    )
}

fn widget_config(widget_uid: uuid::Uuid, label: &str) -> web::WidgetConfig {
    web::WidgetConfig {
        widget_uid: widget_uid.to_string(),
        params: fields_one("label", string_value(label)).into(),
        credential_bindings: web::CredentialBindings::default().into(),
    }
}

async fn stored_widget(fixture: &Fixture, scene_id: scene::SceneId) -> scene::Widget {
    fixture.config.read().await.scenes()[&scene_id]
        .widgets
        .values()
        .next()
        .expect("BUG: one widget in the scene")
        .clone()
}

pub(super) async fn add_small_widget(fixture: &Fixture, label: &str) -> scene::Widget {
    add_widget(
        &fixture.service,
        web::AddWidgetRequest {
            scene_id: fixture.scene_id.to_string(),
            position: web::WidgetPosition::default().into(),
            size: web::WidgetSize::Small.into(),
            config: widget_config(fixture.widget_uid, label).into(),
        },
    )
    .await
    .expect("BUG: add a valid widget");
    stored_widget(fixture, fixture.scene_id).await
}

fn update_request(
    fixture: &Fixture,
    widget: &scene::Widget,
    label: &str,
) -> web::UpdateWidgetRequest {
    web::UpdateWidgetRequest {
        scene_id: fixture.scene_id.to_string(),
        id: widget.id.to_string(),
        position: web::WidgetPosition::default().into(),
        size: web::WidgetSize::Small.into(),
        params: fields_one("label", string_value(label)).into(),
        credential_bindings: web::CredentialBindings::default().into(),
    }
}

async fn set_stamp(fixture: &Fixture, widget_id: scene::WidgetId, stamp: Option<semver::Version>) {
    let mut config = fixture.config.write().await;
    let scene = config
        .scenes_mut()
        .get_mut(&fixture.scene_id)
        .expect("BUG: fixture scene");
    scene
        .widgets
        .get_mut(&widget_id)
        .expect("BUG: fixture widget")
        .params_version = stamp;
}

#[tokio::test]
async fn add_widget_stamps_the_installed_version() {
    let (_temp, fixture) = fixture().await;
    let widget = add_small_widget(&fixture, "new").await;
    assert_eq!(
        widget.params_version,
        Some(WIDGET_VERSION),
        "params added on the installed manifest have its shape, so no gate may migrate them"
    );
    fixture.coordinator.shutdown_widget_manager().await;
}

#[tokio::test]
async fn add_fullscreen_scene_stamps_the_installed_version() {
    let (_temp, fixture) = fixture().await;
    let scene_id = add_fullscreen_scene(
        &fixture.service,
        web::AddFullscreenSceneRequest {
            config: widget_config(fixture.widget_uid, "new").into(),
        },
    )
    .await
    .expect("BUG: add a valid fullscreen scene")
    .body
    .value;
    let scene_id: scene::SceneId = scene_id
        .parse::<uuid::Uuid>()
        .expect("BUG: scene id")
        .into();
    let widget = stored_widget(&fixture, scene_id).await;
    assert_eq!(
        widget.params_version,
        Some(WIDGET_VERSION),
        "a fullscreen scene adds its widget on the installed manifest, like an add"
    );
    fixture.coordinator.shutdown_widget_manager().await;
}

#[tokio::test]
async fn update_with_changed_values_moves_the_stamp() {
    let (_temp, fixture) = fixture().await;
    let widget = add_small_widget(&fixture, "old").await;
    set_stamp(&fixture, widget.id, None).await;

    update_widget(&fixture.service, update_request(&fixture, &widget, "new"))
        .await
        .expect("BUG: valid update");

    let stored = stored_widget(&fixture, fixture.scene_id).await;
    assert_eq!(
        stored.params_version,
        Some(WIDGET_VERSION),
        "changed values were validated against the installed manifest, so they have its shape"
    );
    fixture.coordinator.shutdown_widget_manager().await;
}

#[tokio::test]
async fn update_with_equal_values_keeps_the_stamp() {
    let (_temp, fixture) = fixture().await;
    let widget = add_small_widget(&fixture, "old").await;
    let older = semver::Version::new(0, 9, 0);
    set_stamp(&fixture, widget.id, Some(older.clone())).await;

    update_widget(&fixture.service, update_request(&fixture, &widget, "old"))
        .await
        .expect("BUG: valid no-op update");

    let stored = stored_widget(&fixture, fixture.scene_id).await;
    assert_eq!(
        stored.params_version,
        Some(older),
        "a save that changes no value must neither clear the stamp nor claim the current shape"
    );
    assert_eq!(stored.params, widget.params);
    fixture.coordinator.shutdown_widget_manager().await;
}
