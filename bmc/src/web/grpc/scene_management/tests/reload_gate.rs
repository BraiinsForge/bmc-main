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

//! A scene edit waits for a package reload or boot gate, then gives up.

use super::params_version::{Fixture, add_small_widget, fixture, widget_config};
use super::{
    GrpcSceneManagementService, SceneManagementService, add_fullscreen_scene, add_widget,
    clone_scene, fields_one, no_session, preview_scene, remove_scene, set_scene_cycling,
    string_value, update_scene, update_widget, wire_request,
};
use crate::scene;
use bmc_grpc::web;
use buffa_types::google::protobuf::{Empty, StringValue};
use connectrpc::{ErrorCode, ServiceResult};

#[tokio::test(start_paused = true)]
async fn update_widget_is_unavailable_once_a_gate_held_the_scenes_past_the_bound() {
    let (_temp, fixture) = fixture().await;
    let _gate = fixture.coordinator.hold_scene_edits().await;
    let request = web::UpdateWidgetRequest {
        scene_id: fixture.scene_id.to_string(),
        id: scene::WidgetId::generate().to_string(),
        position: web::WidgetPosition::default().into(),
        size: web::WidgetSize::Small.into(),
        params: fields_one("label", string_value("new")).into(),
        credential_bindings: web::CredentialBindings::default().into(),
    };

    let error = update_widget(&fixture.service, request)
        .await
        .expect_err("the held gate refuses the edit before it looks the widget up");

    assert_eq!(error.code, ErrorCode::Unavailable);
    fixture.coordinator.shutdown_widget_manager().await;
}

fn assert_unavailable<T>(rpc: &str, result: ServiceResult<T>) {
    assert_eq!(
        result.err().map(|error| error.code),
        Some(ErrorCode::Unavailable),
        "{rpc} must wait for the gate, or it lands between the registry swap and the instance replacement"
    );
}

async fn add_combined_scene(service: &SceneManagementService) -> ServiceResult<StringValue> {
    wire_request!(request, Empty::default());
    service.add_combined_scene(no_session(), request).await
}

async fn move_scene(service: &SceneManagementService, scene_id: &str) -> ServiceResult<Empty> {
    wire_request!(
        request,
        web::MoveSceneRequest {
            id: scene_id.to_owned(),
            index: 0,
        }
    );
    service.move_scene(no_session(), request).await
}

async fn remove_widget(
    service: &SceneManagementService,
    scene_id: &str,
    widget_id: scene::WidgetId,
) -> ServiceResult<Empty> {
    wire_request!(
        request,
        web::RemoveWidgetRequest {
            scene_id: scene_id.to_owned(),
            id: widget_id.to_string(),
        }
    );
    service.remove_widget(no_session(), request).await
}

fn valid_add_fullscreen_scene(fixture: &Fixture) -> web::AddFullscreenSceneRequest {
    web::AddFullscreenSceneRequest {
        config: widget_config(fixture.widget_uid, "new").into(),
    }
}

fn valid_update_scene(scene_id: &str) -> web::UpdateSceneRequest {
    web::UpdateSceneRequest {
        id: scene_id.to_owned(),
        enabled: true,
        cycle_duration_sec: None,
    }
}

fn valid_scene_cycling() -> web::SetSceneCyclingRequest {
    web::SetSceneCyclingRequest {
        scene_cycling: web::SceneCycling {
            automatic_cycling_enabled: false,
            automatic_cycling_default_duration_sec: 30,
            transition: web::SceneCyclingTransition::Slide.into(),
        }
        .into(),
    }
}

fn valid_add_widget(fixture: &Fixture) -> web::AddWidgetRequest {
    web::AddWidgetRequest {
        scene_id: fixture.scene_id.to_string(),
        position: web::WidgetPosition::default().into(),
        size: web::WidgetSize::Small.into(),
        config: widget_config(fixture.widget_uid, "new").into(),
    }
}

/// Each request is valid, so only the gate can refuse it.
#[tokio::test(start_paused = true)]
async fn every_scene_edit_is_unavailable_while_a_gate_holds_the_scenes() {
    let (_temp, fixture) = fixture().await;
    let service: &SceneManagementService = &fixture.service;
    let scene_id = fixture.scene_id.to_string();
    let widget = add_small_widget(&fixture, "old").await;
    let _gate = fixture.coordinator.hold_scene_edits().await;

    assert_unavailable(
        "AddFullscreenScene",
        add_fullscreen_scene(service, valid_add_fullscreen_scene(&fixture)).await,
    );
    assert_unavailable("AddCombinedScene", add_combined_scene(service).await);
    assert_unavailable(
        "UpdateScene",
        update_scene(service, valid_update_scene(&scene_id)).await,
    );
    assert_unavailable("MoveScene", move_scene(service, &scene_id).await);
    assert_unavailable("CloneScene", clone_scene(service, &scene_id).await);
    assert_unavailable("RemoveScene", remove_scene(service, &scene_id).await);
    assert_unavailable("PreviewScene", preview_scene(service, &scene_id).await);
    assert_unavailable(
        "RemoveWidget",
        remove_widget(service, &scene_id, widget.id).await,
    );
    assert_unavailable(
        "SetSceneCycling",
        set_scene_cycling(service, valid_scene_cycling()).await,
    );
    assert_unavailable(
        "AddWidget",
        add_widget(service, valid_add_widget(&fixture)).await,
    );

    fixture.coordinator.shutdown_widget_manager().await;
}
