// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::BmcManager;
use crate::config::ConfigHandle;
use crate::countdown_types::{CountdownCompletionAction, LedSettings, SoundSettings};
use crate::led::LedController;
use crate::sound::Sounds;
use crate::web::grpc::GrpcError;
use crate::widget_tasks::WidgetTasks;
use bmc_display::data::{
    AccountId, AddWidgetError, BlockHeightWidget, BraiinsPoolWidget, ClockStyle, ClockWidget,
    CountdownWidget, FontStyle, PoolChartTimeFrame, PoolStyle, RemoteImageWidget, RemoteWidget,
    RemoteWidgetMetadata, RemoveWidgetError, Scene, SceneCycling, SceneCyclingTransition, SceneId,
    SceneKind, TickerBtcWidget, TickerTimeFrame, UpdateWidgetError, Widget, WidgetId, WidgetKind,
    WidgetPosition, WidgetSize,
};
use bmc_display::display_controller::DisplayController;
use bmc_grpc::web;
use bmc_led::data::LedEvent;
use bmc_led::data::{LedEffectKind, Rgb};
use bmc_shared_time::time::Timezone;
use futures::StreamExt;
use futures::stream::BoxStream;
use prost_types::{ListValue, Struct, Value as ProstValue, value::Kind as ProstKind};
use reqwest::Client;
use serde_json::Value as JsonValue;
use std::collections::HashSet;
use std::fmt::Display;
use std::panic;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tap::{TapFallible, TapOptional};
use tokio::sync::{Mutex, RwLock};
use tokio::time;
use tokio_stream::wrappers::IntervalStream;
use tonic::{Request, Response, Status};
use tonic_types::{ErrorDetails, FieldViolation, StatusExt};
use tooling_std::attach_data::AttachData;
use tracing::{error, warn};
use url::Url;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const RECENT_REMOTE_WIDGETS: usize = 3;

pub(crate) struct SceneManagementService<T: BmcManager> {
    config_handle: Arc<RwLock<ConfigHandle>>,
    display_controller: DisplayController,
    widget_tasks: WidgetTasks,
    preview_scene_id: Arc<Mutex<Option<SceneId>>>,
    led_controller: LedController<T>,
}

impl<T: BmcManager> SceneManagementService<T> {
    pub(crate) fn new(
        config_handle: Arc<RwLock<ConfigHandle>>,
        display_controller: DisplayController,
        widget_tasks: WidgetTasks,
        led_controller: LedController<T>,
    ) -> Self {
        Self {
            config_handle,
            display_controller,
            widget_tasks,
            preview_scene_id: Arc::default(),
            led_controller,
        }
    }
}

#[async_trait::async_trait]
impl<T: BmcManager> web::scene_management_service_server::SceneManagementService
    for SceneManagementService<T>
{
    async fn get_scenes(
        &self,
        _request: Request<()>,
    ) -> Result<Response<web::GetScenesResponse>, Status> {
        let config = self.config_handle.read().await;
        let scenes = config
            .scenes
            .clone()
            .into_values()
            .map(map_scene_to_proto)
            .collect();

        Ok(Response::new(web::GetScenesResponse { scenes }))
    }

    async fn get_scene(
        &self,
        request: Request<String>,
    ) -> Result<Response<web::SceneResponse>, Status> {
        let value = request.into_inner();

        let (id, field_violations) = parse_scene_id("value", &value);

        if !field_violations.is_empty() {
            return Err(Status::with_error_details(
                tonic::Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request(field_violations),
            ));
        }

        let id = id.ok_or_else(unchecked_field_violations_status)?;

        let config = self.config_handle.read().await;
        let scene = config
            .scenes
            .get(&id)
            .cloned()
            .map(map_scene_to_proto)
            .ok_or_else(|| Status::not_found("Scene not found"))?;

        Ok(Response::new(web::SceneResponse { scene: Some(scene) }))
    }

    async fn add_fullscreen_scene(
        &self,
        request: Request<web::AddFullscreenSceneRequest>,
    ) -> Result<Response<String>, Status> {
        let request = request.into_inner();

        let (widget_kind, field_violations) =
            parse_widget_kind_with_default_params("widget_kind", request.widget_kind).await;

        if !field_violations.is_empty() {
            return Err(Status::with_error_details(
                tonic::Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request(field_violations),
            ));
        }

        let widget_kind = widget_kind.ok_or_else(unchecked_field_violations_status)?;

        // NOTE: wrapped in tokio task to avoid cancellation on client disconnect
        let join_handle = tokio::spawn({
            let config_handle = self.config_handle.clone();
            let display_controller = self.display_controller.clone();
            let widget_tasks = self.widget_tasks.clone();
            async move {
                let new_scene = Scene::fullscreen(widget_kind);
                let new_scene_id = new_scene.id.clone();

                {
                    let mut config = config_handle.write().await;
                    let mut temp_config = config.clone();

                    let replaced_scene = temp_config
                        .scenes
                        .insert(new_scene_id.clone(), new_scene.clone());
                    debug_assert!(replaced_scene.is_none());

                    if let Err(err) = temp_config.save().await {
                        error!("Cannot save config: {}", err);
                        return Err(Status::internal("Failed to save configuration"));
                    }
                    *config = temp_config;
                }

                if new_scene.enabled {
                    widget_tasks
                        .spawn_all(&new_scene.id, new_scene.widgets.values())
                        .await;
                }

                display_controller.add_scene(new_scene);

                Ok(Response::new(new_scene_id.to_string()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }

    async fn add_combined_scene(&self, _request: Request<()>) -> Result<Response<String>, Status> {
        // NOTE: wrapped in tokio task to avoid cancellation on client disconnect
        let join_handle = tokio::spawn({
            let config_handle = self.config_handle.clone();
            let display_controller = self.display_controller.clone();
            let widget_tasks = self.widget_tasks.clone();
            async move {
                let new_scene = Scene::combined();
                let new_scene_id = new_scene.id.clone();

                {
                    let mut config = config_handle.write().await;
                    let mut temp_config = config.clone();

                    let replaced_scene = temp_config
                        .scenes
                        .insert(new_scene_id.clone(), new_scene.clone());
                    debug_assert!(replaced_scene.is_none());

                    if let Err(err) = temp_config.save().await {
                        error!("Cannot save config: {}", err);
                        return Err(Status::internal("Failed to save configuration"));
                    }
                    *config = temp_config;
                }

                if new_scene.enabled {
                    widget_tasks
                        .spawn_all(&new_scene.id, new_scene.widgets.values())
                        .await;
                }

                display_controller.add_scene(new_scene);

                Ok(Response::new(new_scene_id.to_string()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }

    async fn update_scene(
        &self,
        request: Request<web::UpdateSceneRequest>,
    ) -> Result<Response<()>, Status> {
        let request = request.into_inner();
        let mut all_field_violations = FieldViolations::new();

        let (id, field_violations) = parse_scene_id("id", &request.id);
        all_field_violations.extend(field_violations);

        let (cycle_duration, field_violations) =
            parse_scene_cycle_duration("cycle_duration_sec", request.cycle_duration_sec);
        all_field_violations.extend(field_violations);

        let enabled = request.enabled;

        if !all_field_violations.is_empty() {
            return Err(Status::with_error_details(
                tonic::Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request(all_field_violations),
            ));
        }

        let id = id.ok_or_else(unchecked_field_violations_status)?;
        let cycle_duration = cycle_duration.ok_or_else(unchecked_field_violations_status)?;

        // NOTE: wrapped in tokio task to avoid cancellation on client disconnect
        let join_handle = tokio::spawn({
            let config_handle = self.config_handle.clone();
            let display_controller = self.display_controller.clone();
            let widget_tasks = self.widget_tasks.clone();
            async move {
                let previously_enabled = {
                    let mut config = config_handle.write().await;
                    let mut temp_config = config.clone();

                    let scene = temp_config
                        .scenes
                        .get_mut(&id)
                        .ok_or_else(|| Status::not_found("Scene not found"))?;

                    let previously_enabled = scene.enabled;

                    scene.enabled = enabled;
                    scene.cycle_duration = cycle_duration;

                    if let Err(err) = temp_config.save().await {
                        error!("Cannot save config: {}", err);
                        return Err(Status::internal("Failed to save configuration"));
                    }
                    *config = temp_config;
                    previously_enabled
                };

                if enabled != previously_enabled {
                    let config = config_handle.read().await;
                    let scene = &config.scenes[&id];

                    if previously_enabled {
                        widget_tasks.abort_all(&scene.id).await;
                    }

                    if enabled {
                        widget_tasks
                            .spawn_all(&scene.id, scene.widgets.values())
                            .await;
                    }

                    // NOTE: we need to reset cycler, because this operation could move scene to another index
                    display_controller.reset_cycler();
                }

                display_controller.update_scene(id, enabled, cycle_duration);

                Ok(Response::new(()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }

    async fn move_scene(
        &self,
        request: Request<web::MoveSceneRequest>,
    ) -> Result<Response<()>, Status> {
        let request = request.into_inner();
        let mut all_field_violations = FieldViolations::new();

        let (id, field_violations) = parse_scene_id("id", &request.id);
        all_field_violations.extend(field_violations);

        let to_index = request.index as usize;

        if !all_field_violations.is_empty() {
            return Err(Status::with_error_details(
                tonic::Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request(all_field_violations),
            ));
        }

        let id = id.ok_or_else(unchecked_field_violations_status)?;

        // NOTE: wrapped in tokio task to avoid cancellation on client disconnect
        let join_handle = tokio::spawn({
            let config_handle = self.config_handle.clone();
            let display_controller = self.display_controller.clone();
            let preview_scene_id = self.preview_scene_id.clone();
            async move {
                let (from_index, to_index) = {
                    let mut config = config_handle.write().await;
                    let mut temp_config = config.clone();

                    let from_index = temp_config
                        .scenes
                        .get_index_of(&id)
                        .ok_or_else(|| Status::not_found("Scene not found"))?;

                    let to_index = to_index.min(temp_config.scenes.len() - 1);

                    if from_index == to_index {
                        return Ok(Response::new(()));
                    }
                    temp_config.scenes.move_index(from_index, to_index);

                    if let Err(err) = temp_config.save().await {
                        error!("Cannot save config: {}", err);
                        return Err(Status::internal("Failed to save configuration"));
                    }
                    *config = temp_config;
                    (from_index, to_index)
                };

                display_controller.move_scene(from_index, to_index);

                // NOTE: we need to reset cycler, because this operation could move scene to another index
                display_controller.reset_cycler();

                // NOTE: we need to re-focus preview scene, because this operation could move scene to another index
                display_controller.set_preview_scene(preview_scene_id.lock().await.clone());

                Ok(Response::new(()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }

    async fn clone_scene(&self, request: Request<String>) -> Result<Response<String>, Status> {
        let value = request.into_inner();

        let (id, field_violations) = parse_scene_id("value", &value);

        if !field_violations.is_empty() {
            return Err(Status::with_error_details(
                tonic::Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request(field_violations),
            ));
        }

        let id = id.ok_or_else(unchecked_field_violations_status)?;

        // NOTE: wrapped in tokio task to avoid cancellation on client disconnect
        let join_handle = tokio::spawn({
            let config_handle = self.config_handle.clone();
            let display_controller = self.display_controller.clone();
            let widget_tasks = self.widget_tasks.clone();
            let preview_scene_id = self.preview_scene_id.clone();
            async move {
                let (cloned_scene, cloned_scene_index) = {
                    let mut config = config_handle.write().await;
                    let mut temp_config = config.clone();

                    let (scene_index, _id, scene) = temp_config
                        .scenes
                        .get_full(&id)
                        .ok_or_else(|| Status::not_found("Scene not found"))?;

                    let cloned_scene = scene.clone_with_new_id();
                    let cloned_scene_id = cloned_scene.id.clone();
                    let cloned_scene_index = scene_index + 1;

                    let replaced_scene = temp_config.scenes.shift_insert(
                        cloned_scene_index,
                        cloned_scene_id,
                        cloned_scene.clone(),
                    );
                    debug_assert!(replaced_scene.is_none());

                    if let Err(err) = temp_config.save().await {
                        error!("Cannot save config: {}", err);
                        return Err(Status::internal("Failed to save configuration"));
                    }
                    *config = temp_config;
                    (cloned_scene, cloned_scene_index)
                };

                let cloned_scene_id = cloned_scene.id.clone();

                if cloned_scene.enabled {
                    widget_tasks
                        .spawn_all(&cloned_scene.id, cloned_scene.widgets.values())
                        .await;
                }

                display_controller.insert_scene(cloned_scene_index, cloned_scene);

                // NOTE: we need to reset cycler, because this operation could move scene to another index
                display_controller.reset_cycler();

                // NOTE: we need to re-focus preview scene, because this operation could move scene to another index
                display_controller.set_preview_scene(preview_scene_id.lock().await.clone());

                Ok(Response::new(cloned_scene_id.to_string()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }

    async fn remove_scene(&self, request: Request<String>) -> Result<Response<()>, Status> {
        let value = request.into_inner();

        let (id, field_violations) = parse_scene_id("value", &value);

        if !field_violations.is_empty() {
            return Err(Status::with_error_details(
                tonic::Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request(field_violations),
            ));
        }

        let id = id.ok_or_else(unchecked_field_violations_status)?;

        {
            let preview_scene_id = self.preview_scene_id.lock().await;

            if preview_scene_id
                .as_ref()
                .is_some_and(|preview_id| *preview_id == id)
            {
                return Err(Status::failed_precondition(
                    "Scene is currently displayed in preview",
                ));
            }
        }

        // NOTE: wrapped in tokio task to avoid cancellation on client disconnect
        let join_handle = tokio::spawn({
            let config_handle = self.config_handle.clone();
            let display_controller = self.display_controller.clone();
            let widget_tasks = self.widget_tasks.clone();
            let preview_scene_id = self.preview_scene_id.clone();
            async move {
                let scene = {
                    let mut config = config_handle.write().await;
                    let mut temp_config = config.clone();

                    let scene = temp_config
                        .scenes
                        .shift_remove(&id)
                        .ok_or_else(|| Status::not_found("Scene not found"))?;

                    if let Err(err) = temp_config.save().await {
                        error!("Cannot save config: {}", err);
                        return Err(Status::internal("Failed to save configuration"));
                    }
                    *config = temp_config;
                    scene
                };

                if scene.enabled {
                    widget_tasks.abort_all(&scene.id).await;
                }

                display_controller.remove_scene(id);

                // NOTE: we need to reset cycler, because this operation could move scene to another index
                display_controller.reset_cycler();

                // NOTE: we need to re-focus preview scene, because this operation could move scene to another index
                display_controller.set_preview_scene(preview_scene_id.lock().await.clone());

                Ok(Response::new(()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }

    type PreviewSceneStream = BoxStream<'static, Result<(), Status>>;

    async fn preview_scene(
        &self,
        request: Request<String>,
    ) -> Result<Response<Self::PreviewSceneStream>, Status> {
        let value = request.into_inner();

        let (id, field_violations) = parse_scene_id("value", &value);

        if !field_violations.is_empty() {
            return Err(Status::with_error_details(
                tonic::Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request(field_violations),
            ));
        }

        let id = id.ok_or_else(unchecked_field_violations_status)?;

        let started_temporary_widget_tasks = {
            let config = self.config_handle.read().await;
            let scene = config
                .scenes
                .get(&id)
                .ok_or_else(|| Status::not_found("Scene not found"))?;

            {
                let mut preview_scene_id = self.preview_scene_id.lock().await;
                if preview_scene_id.is_some() {
                    return Err(Status::resource_exhausted(
                        "Scene preview is already enabled",
                    ));
                }
                *preview_scene_id = Some(id.clone());
            }

            // we want to start temporary widget tasks if scene is disabled, otherwise we won't have
            // data to properly render preview
            if scene.enabled {
                false
            } else {
                self.widget_tasks
                    .spawn_all(&scene.id, scene.widgets.values())
                    .await;

                true
            }
        };

        self.display_controller.set_preview_scene(Some(id.clone()));
        self.led_controller.push_event(LedEvent::PreviewScene);

        struct DisableScenePreviewOnDrop<T: BmcManager> {
            display_controller: DisplayController,
            widget_tasks: WidgetTasks,
            preview_scene_id: Arc<Mutex<Option<SceneId>>>,
            started_temporary_widget_tasks: bool,
            led_controller: LedController<T>,
        }

        impl<T: BmcManager> Drop for DisableScenePreviewOnDrop<T> {
            fn drop(&mut self) {
                self.display_controller.set_preview_scene(None);
                self.led_controller.push_event(LedEvent::PreviewSceneEnded);

                tokio::spawn({
                    let preview_scene_id = self.preview_scene_id.clone();
                    let widget_tasks = self.widget_tasks.clone();
                    let started_temporary_widget_tasks = self.started_temporary_widget_tasks;
                    async move {
                        let scene_id = preview_scene_id
                            .lock()
                            .await
                            .take()
                            .expect("BUG: preview_scene_id should be set");

                        // we need to stop temporary widget tasks, because they shouldn't be running
                        // outside of preview since scene is disabled
                        if started_temporary_widget_tasks {
                            widget_tasks.abort_all(&scene_id).await;
                        }
                    }
                });
            }
        }

        let stream = IntervalStream::new(time::interval(Duration::from_secs(5)))
            .map(|_| Ok(()))
            .attach_data(DisableScenePreviewOnDrop {
                display_controller: self.display_controller.clone(),
                widget_tasks: self.widget_tasks.clone(),
                preview_scene_id: self.preview_scene_id.clone(),
                started_temporary_widget_tasks,
                led_controller: self.led_controller.clone(),
            })
            .boxed();

        Ok(Response::new(stream))
    }

    async fn add_widget(
        &self,
        request: Request<web::AddWidgetRequest>,
    ) -> Result<Response<String>, Status> {
        let request = request.into_inner();
        let mut all_field_violations = FieldViolations::new();

        let (scene_id, field_violations) = parse_scene_id("scene_id", &request.scene_id);
        all_field_violations.extend(field_violations);

        let (position, field_violations) = parse_widget_position("position", request.position);
        all_field_violations.extend(field_violations);

        let (size, field_violations) = parse_widget_size("size", request.size());
        all_field_violations.extend(field_violations);

        let (kind, field_violations) =
            parse_widget_kind_with_default_params("kind", request.kind).await;
        all_field_violations.extend(field_violations);

        if !all_field_violations.is_empty() {
            return Err(Status::with_error_details(
                tonic::Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request(all_field_violations),
            ));
        }

        let scene_id = scene_id.ok_or_else(unchecked_field_violations_status)?;
        let position = position.ok_or_else(unchecked_field_violations_status)?;
        let size = size.ok_or_else(unchecked_field_violations_status)?;
        let kind = kind.ok_or_else(unchecked_field_violations_status)?;

        // NOTE: wrapped in tokio task to avoid cancellation on client disconnect
        let join_handle = tokio::spawn({
            let config_handle = self.config_handle.clone();
            let display_controller = self.display_controller.clone();
            let widget_tasks = self.widget_tasks.clone();
            let preview_scene_id = self.preview_scene_id.clone();
            async move {
                let (new_widget, scene_enabled) = {
                    let mut config = config_handle.write().await;
                    let mut temp_config = config.clone();

                    let scene = temp_config
                        .scenes
                        .get_mut(&scene_id)
                        .ok_or_else(|| Status::not_found("Scene not found"))?;

                    let new_widget =
                        scene
                            .add_widget(position, size, kind)
                            .map_err(|err| match err {
                                AddWidgetError::CannotAddWidgetToFullscreenScene
                                | AddWidgetError::CannotAddFullscreenWidgetToCombinedScene
                                | AddWidgetError::InvalidWidgetPlacement(_) => {
                                    Status::failed_precondition(err.to_string())
                                }
                            })?;

                    let scene_enabled = scene.enabled;

                    if let Err(err) = temp_config.save().await {
                        error!("Cannot save config: {}", err);
                        return Err(Status::internal("Failed to save configuration"));
                    }
                    *config = temp_config;
                    (new_widget, scene_enabled)
                };

                let new_widget_id = new_widget.id.clone();

                let is_preview_scene = preview_scene_id
                    .lock()
                    .await
                    .as_ref()
                    .is_some_and(|preview_scene_id| *preview_scene_id == scene_id);

                if scene_enabled || is_preview_scene {
                    widget_tasks.spawn(&scene_id, &new_widget).await;
                }

                display_controller.add_scene_widget(scene_id, new_widget);

                Ok(Response::new(new_widget_id.to_string()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }

    async fn update_widget(
        &self,
        request: Request<web::UpdateWidgetRequest>,
    ) -> Result<Response<()>, Status> {
        let request = request.into_inner();
        let mut all_field_violations = FieldViolations::new();

        let (scene_id, field_violations) = parse_scene_id("scene_id", &request.scene_id);
        all_field_violations.extend(field_violations);

        let (widget_id, field_violations) = parse_widget_id("id", &request.id);
        all_field_violations.extend(field_violations);

        let (position, field_violations) = parse_widget_position("position", request.position);
        all_field_violations.extend(field_violations);

        let (size, field_violations) = parse_widget_size("size", request.size());
        all_field_violations.extend(field_violations);

        let (kind, field_violations) = parse_widget_kind("kind", request.kind);
        all_field_violations.extend(field_violations);

        if !all_field_violations.is_empty() {
            return Err(Status::with_error_details(
                tonic::Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request(all_field_violations),
            ));
        }

        let scene_id = scene_id.ok_or_else(unchecked_field_violations_status)?;
        let widget_id = widget_id.ok_or_else(unchecked_field_violations_status)?;
        let position = position.ok_or_else(unchecked_field_violations_status)?;
        let size = size.ok_or_else(unchecked_field_violations_status)?;
        let kind = kind.ok_or_else(unchecked_field_violations_status)?;

        // NOTE: wrapped in tokio task to avoid cancellation on client disconnect
        let join_handle = tokio::spawn({
            let config_handle = self.config_handle.clone();
            let display_controller = self.display_controller.clone();
            let widget_tasks = self.widget_tasks.clone();
            let preview_scene_id = self.preview_scene_id.clone();
            async move {
                let (updated_widget, scene_enabled) = {
                    let mut config = config_handle.write().await;
                    let mut temp_config = config.clone();

                    let scene = temp_config
                        .scenes
                        .get_mut(&scene_id)
                        .ok_or_else(|| Status::not_found("Scene not found"))?;

                    let updated_widget = scene
                        .update_widget(&widget_id, position, size, kind)
                        .map_err(|err| match err {
                            UpdateWidgetError::NotFound => Status::not_found(err.to_string()),
                            UpdateWidgetError::CannotUpdateWidgetPositionInFullscreenScene
                            | UpdateWidgetError::CannotUpdateWidgetSizeInFullscreenScene
                            | UpdateWidgetError::CannotUpdateWidgetSizeToFullInCombinedScene
                            | UpdateWidgetError::CannotSwitchWidgetKind
                            | UpdateWidgetError::InvalidWidgetPlacement(_) => {
                                Status::failed_precondition(err.to_string())
                            }
                        })?;

                    let scene_enabled = scene.enabled;

                    if let Err(err) = temp_config.save().await {
                        error!("Cannot save config: {}", err);
                        return Err(Status::internal("Failed to save configuration"));
                    }
                    *config = temp_config;
                    (updated_widget, scene_enabled)
                };

                let is_preview_scene = preview_scene_id
                    .lock()
                    .await
                    .as_ref()
                    .is_some_and(|preview_scene_id| *preview_scene_id == scene_id);

                if scene_enabled || is_preview_scene {
                    widget_tasks.abort(&scene_id, &updated_widget.id).await;

                    widget_tasks.spawn(&scene_id, &updated_widget).await;
                }

                display_controller.replace_scene_widget(scene_id, updated_widget);

                Ok(Response::new(()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }

    async fn remove_widget(
        &self,
        request: Request<web::RemoveWidgetRequest>,
    ) -> Result<Response<()>, Status> {
        let request = request.into_inner();
        let mut all_field_violations = FieldViolations::new();

        let (widget_id, field_violations) = parse_widget_id("id", &request.id);
        all_field_violations.extend(field_violations);

        let (scene_id, field_violations) = parse_scene_id("scene_id", &request.scene_id);
        all_field_violations.extend(field_violations);

        if !all_field_violations.is_empty() {
            return Err(Status::with_error_details(
                tonic::Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request(all_field_violations),
            ));
        }

        let widget_id = widget_id.ok_or_else(unchecked_field_violations_status)?;
        let scene_id = scene_id.ok_or_else(unchecked_field_violations_status)?;

        // NOTE: wrapped in tokio task to avoid cancellation on client disconnect
        let join_handle = tokio::spawn({
            let config_handle = self.config_handle.clone();
            let display_controller = self.display_controller.clone();
            let widget_tasks = self.widget_tasks.clone();
            async move {
                let scene_enabled = {
                    let mut config = config_handle.write().await;
                    let mut temp_config = config.clone();

                    let scene = temp_config
                        .scenes
                        .get_mut(&scene_id)
                        .ok_or_else(|| Status::not_found("Scene not found"))?;

                    let scene_enabled = scene.enabled;

                    scene.remove_widget(&widget_id).map_err(|err| match err {
                        RemoveWidgetError::NotFound => Status::not_found(err.to_string()),
                        RemoveWidgetError::CannotRemoveWidgetFromFullscreenScene => {
                            Status::failed_precondition(err.to_string())
                        }
                    })?;

                    if let Err(err) = temp_config.save().await {
                        error!("Cannot save config: {}", err);
                        return Err(Status::internal("Failed to save configuration"));
                    }
                    *config = temp_config;
                    scene_enabled
                };

                if scene_enabled {
                    widget_tasks.abort(&scene_id, &widget_id).await;
                }

                display_controller.remove_scene_widget(scene_id, widget_id);

                Ok(Response::new(()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }

    async fn get_remote_widget_params(
        &self,
        request: Request<web::RemoteWidgetParamsRequest>,
    ) -> Result<Response<web::RemoteWidgetParamsResponse>, Status> {
        let request = request.into_inner();

        let mut parsed_url = match Url::parse(&request.widget_url) {
            Ok(url) => url,
            Err(err) => {
                warn!(?err, "Invalid URL");
                return Err(Status::invalid_argument("Invalid URL"));
            }
        };

        match parsed_url.path_segments_mut() {
            Ok(mut path_segments) => path_segments.push("metadata"),
            Err(err) => {
                warn!(?err, "Failed to get path segments");
                return Err(Status::internal("Failed to get path segment"));
            }
        };

        let client = match Client::builder().timeout(REQUEST_TIMEOUT).build() {
            Ok(client) => client,
            Err(err) => {
                warn!(?err, "Failed to create reqwest client");
                return Err(Status::internal("Failed to create reqwest client"));
            }
        };

        let metadata_response = match client.get(parsed_url).send().await {
            Ok(response) => {
                if !response.status().is_success() {
                    warn!("Failed to get metadata");
                    return Err(Status::failed_precondition("Failed to get metadata"));
                }
                response
            }
            Err(err) => {
                warn!(?err, "Failed to get metadata");
                return Err(Status::failed_precondition("Failed to get metadata"));
            }
        };

        let metadata = match metadata_response.json::<RemoteWidgetMetadata>().await {
            Ok(metadata) => metadata,
            Err(err) => {
                warn!(?err, "Failed to parse metadata");
                return Err(Status::failed_precondition("Failed to parse metadata"));
            }
        };

        let response = web::RemoteWidgetParamsResponse {
            remote_widget_params: Some(map_params_to_protobuf_struct(metadata.params)),
        };

        Ok(Response::new(response))
    }

    async fn get_recent_remote_widgets(
        &self,
        _request: Request<()>,
    ) -> Result<Response<web::RecentRemoteWidgetsResponse>, Status> {
        let config = self.config_handle.read().await;
        let mut unique = HashSet::new();
        let recent_remote_widgets: Vec<web::RemoteWidget> = config
            .scenes
            .values()
            .flat_map(|scene| scene.widgets.values().cloned())
            .filter_map(|w| {
                if let WidgetKind::RemoteWidget(remote_widget) = w.kind {
                    Some(remote_widget)
                } else {
                    None
                }
            })
            .filter(|remote_widget| unique.insert(remote_widget.name.clone()))
            .rev()
            .take(RECENT_REMOTE_WIDGETS)
            .map(map_remote_widget_to_proto_remote_widget)
            .collect();

        Ok(Response::new(web::RecentRemoteWidgetsResponse {
            recent_remote_widgets,
        }))
    }

    async fn get_scene_cycling(
        &self,
        _request: Request<()>,
    ) -> Result<Response<web::GetSceneCyclingResponse>, Status> {
        let config = self.config_handle.read().await;
        let scene_cycling = map_scene_cycling_to_proto(config.scene_cycling());

        Ok(Response::new(web::GetSceneCyclingResponse {
            scene_cycling: Some(scene_cycling),
        }))
    }

    async fn set_scene_cycling(
        &self,
        request: Request<web::SetSceneCyclingRequest>,
    ) -> Result<Response<()>, Status> {
        let request = request.into_inner();
        let mut all_field_violations = FieldViolations::new();

        let (scene_cycling, field_violations) =
            parse_scene_cycling("scene_cycling", request.scene_cycling);
        all_field_violations.extend(field_violations);

        if !all_field_violations.is_empty() {
            return Err(Status::with_error_details(
                tonic::Code::InvalidArgument,
                GrpcError::BadRequest.to_string(),
                ErrorDetails::with_bad_request(all_field_violations),
            ));
        }

        let scene_cycling = scene_cycling.ok_or_else(unchecked_field_violations_status)?;

        // NOTE: wrapped in tokio task to avoid cancellation on client disconnect
        let join_handle = tokio::spawn({
            let config_handle = self.config_handle.clone();
            let display_controller = self.display_controller.clone();
            async move {
                let (previously_enabled, enabled) = {
                    let mut config = config_handle.write().await;
                    let mut temp_config = config.clone();

                    let previously_enabled = temp_config.scene_cycling().automatic_cycling_enabled;
                    let enabled = scene_cycling.automatic_cycling_enabled;

                    temp_config.set_scene_cycling(scene_cycling.clone());

                    if let Err(err) = temp_config.save().await {
                        error!("Cannot save config: {}", err);
                        return Err(Status::internal("Failed to save configuration"));
                    }
                    *config = temp_config;
                    (previously_enabled, enabled)
                };

                display_controller.set_scene_cycling(scene_cycling);

                if enabled != previously_enabled {
                    display_controller.reset_cycler();
                }

                Ok(Response::new(()))
            }
        });

        join_handle
            .await
            .unwrap_or_else(|err| panic::resume_unwind(err.into_panic()))
    }
}

type ParseOutput<T> = (Option<T>, FieldViolations);

struct FieldViolations(Vec<FieldViolation>);

impl FieldViolations {
    pub fn new() -> Self {
        Self(Vec::with_capacity(0))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn push(&mut self, field: impl AsRef<str>, description: impl AsRef<str>) {
        self.0
            .push(FieldViolation::new(field.as_ref(), description.as_ref()));
    }

    pub fn extend(&mut self, other: Self) {
        self.0.extend(other.0);
    }
}

impl From<FieldViolations> for Vec<FieldViolation> {
    fn from(value: FieldViolations) -> Self {
        value.0
    }
}

fn unchecked_field_violations_status() -> Status {
    Status::internal("Unchecked field violations")
}

fn parse_scene_id(field: impl AsRef<str> + Display, input: &str) -> ParseOutput<SceneId> {
    let mut field_violations = FieldViolations::new();

    let maybe_id = SceneId::from_str(input).ok().tap_none(|| {
        field_violations.push(field, "Invalid scene ID!");
    });

    (maybe_id, field_violations)
}

fn parse_widget_id(field: impl AsRef<str> + Display, input: &str) -> ParseOutput<WidgetId> {
    let mut field_violations = FieldViolations::new();

    let maybe_id = WidgetId::from_str(input).ok().tap_none(|| {
        field_violations.push(field, "Invalid widget ID!");
    });

    (maybe_id, field_violations)
}

fn parse_scene_cycle_duration(
    field: impl AsRef<str> + Display,
    input: Option<u32>,
) -> ParseOutput<Option<Duration>> {
    let mut field_violations = FieldViolations::new();

    let Some(input) = input else {
        return (Some(None), field_violations);
    };
    let cycle_duration = Duration::from_secs(input.into());

    if cycle_duration < Scene::MIN_CYCLE_DURATION {
        field_violations.push(
            field,
            format!("Out of range: {}..", Scene::MIN_CYCLE_DURATION.as_secs()),
        );
        (None, field_violations)
    } else {
        (Some(Some(cycle_duration)), field_violations)
    }
}

fn parse_widget_position(
    field: impl AsRef<str> + Display,
    input: Option<web::WidgetPosition>,
) -> ParseOutput<WidgetPosition> {
    let mut field_violations = FieldViolations::new();

    let Some(input) = input else {
        field_violations.push(field, "Missing value!");
        return (None, field_violations);
    };

    let maybe_row = if input.row < WidgetPosition::MAX_ROWS {
        #[expect(clippy::cast_possible_truncation)]
        Some(input.row as u8)
    } else {
        field_violations.push(
            format!("{field}.row"),
            format!("Out of range: 0..{}", WidgetPosition::MAX_ROWS),
        );
        None
    };

    let maybe_col = if input.col < WidgetPosition::MAX_COLS {
        #[expect(clippy::cast_possible_truncation)]
        Some(input.col as u8)
    } else {
        field_violations.push(
            format!("{field}.col"),
            format!("Out of range: 0..{}", WidgetPosition::MAX_COLS),
        );
        None
    };

    let maybe_position = maybe_row
        .zip(maybe_col)
        .map(|(row, col)| WidgetPosition { row, col });

    (maybe_position, field_violations)
}

fn parse_widget_size(
    field: impl AsRef<str> + Display,
    input: web::WidgetSize,
) -> ParseOutput<WidgetSize> {
    let mut field_violations = FieldViolations::new();

    let maybe_size = match input {
        web::WidgetSize::Unspecified => {
            field_violations.push(field, "Missing value!");
            None
        }
        web::WidgetSize::Small => Some(WidgetSize::Small),
        web::WidgetSize::Medium => Some(WidgetSize::Medium),
        web::WidgetSize::Large => Some(WidgetSize::Large),
        web::WidgetSize::Full => Some(WidgetSize::Full),
    };

    (maybe_size, field_violations)
}

async fn parse_widget_kind_with_default_params(
    field: impl AsRef<str> + Display,
    input: Option<web::WidgetKind>,
) -> ParseOutput<WidgetKind> {
    let mut field_violations = FieldViolations::new();

    let Some(input) = input else {
        field_violations.push(field, "Missing value!");
        return (None, field_violations);
    };

    let Some(value) = input.value else {
        field_violations.push(format!("{field}.value"), "Missing value!");
        return (None, field_violations);
    };

    let kind = match value {
        web::widget_kind::Value::Clock(_) => WidgetKind::Clock(ClockWidget::default()),
        web::widget_kind::Value::TickerBtc(_) => WidgetKind::TickerBtc(TickerBtcWidget::default()),
        web::widget_kind::Value::BlockHeight(_) => {
            WidgetKind::BlockHeight(BlockHeightWidget::default())
        }
        web::widget_kind::Value::BraiinsPool(_) => {
            WidgetKind::BraiinsPool(BraiinsPoolWidget::default())
        }
        web::widget_kind::Value::RemoteImage(_) => {
            WidgetKind::RemoteImage(RemoteImageWidget::default())
        }
        web::widget_kind::Value::BlockchainData(_) => WidgetKind::BlockchainData,
        web::widget_kind::Value::RemoteWidget(remote_widget) => {
            match remote_widget_url_validation(&remote_widget, &mut field_violations).await {
                Some(kind) => kind,
                None => return (None, field_violations),
            }
        }
        web::widget_kind::Value::HalvingCountdown(_) => WidgetKind::HalvingCountdown,
        web::widget_kind::Value::Countdown(_) => WidgetKind::Countdown(CountdownWidget::default()),
    };

    (Some(kind), field_violations)
}

fn parse_widget_kind(
    field: impl AsRef<str> + Display,
    input: Option<web::WidgetKind>,
) -> ParseOutput<WidgetKind> {
    let mut all_field_violations = FieldViolations::new();

    let Some(input) = input else {
        all_field_violations.push(field, "Missing value!");
        return (None, all_field_violations);
    };

    let Some(value) = input.value else {
        all_field_violations.push(format!("{field}.value"), "Missing value!");
        return (None, all_field_violations);
    };

    let maybe_kind = match value {
        web::widget_kind::Value::Clock(clock_proto) => {
            let (maybe_kind, field_violations) =
                parse_clock_widget_kind(format!("{field}.clock"), clock_proto);
            all_field_violations.extend(field_violations);
            maybe_kind
        }
        web::widget_kind::Value::TickerBtc(ticker_btc_proto) => {
            let (maybe_kind, field_violations) =
                parse_ticker_btc_widget_kind("ticker_btc", ticker_btc_proto);
            all_field_violations.extend(field_violations);
            maybe_kind
        }
        web::widget_kind::Value::BlockHeight(block_height_proto) => {
            let (maybe_kind, field_violations) =
                parse_block_height_widget_kind("block_height", block_height_proto);
            all_field_violations.extend(field_violations);
            maybe_kind
        }
        web::widget_kind::Value::BraiinsPool(braiins_pool_proto) => {
            let (maybe_kind, field_violations) =
                parse_braiins_pool_widget_kind("braiins_pool", braiins_pool_proto);
            all_field_violations.extend(field_violations);
            maybe_kind
        }
        web::widget_kind::Value::RemoteImage(remote_image_proto) => {
            let (maybe_kind, field_violations) =
                parse_remote_image_widget_kind(format!("{field}.remote_image"), remote_image_proto);
            all_field_violations.extend(field_violations);
            maybe_kind
        }
        web::widget_kind::Value::BlockchainData(_) => Some(WidgetKind::BlockchainData),
        web::widget_kind::Value::RemoteWidget(remote_widget_proto) => {
            Some(map_proto_to_remote_widget_kind(remote_widget_proto))
        }
        web::widget_kind::Value::HalvingCountdown(_) => Some(WidgetKind::HalvingCountdown),
        web::widget_kind::Value::Countdown(countdown_proto) => {
            let (maybe_kind, field_violations) =
                parse_countdown_widget_kind(format!("{field}.countdown"), countdown_proto);
            all_field_violations.extend(field_violations);
            maybe_kind
        }
    };

    (maybe_kind, all_field_violations)
}

fn parse_clock_widget_kind(
    field: impl AsRef<str> + Display,
    clock_proto: web::ClockWidget,
) -> ParseOutput<WidgetKind> {
    use web::FontStyle as FontStyleProto;
    use web::clock_widget::ClockStyle as ClockStyleProto;

    let mut field_violations = FieldViolations::new();

    let maybe_clock_style = match clock_proto.clock_style() {
        ClockStyleProto::Unspecified => {
            field_violations.push(format!("{field}.clock_style"), "Missing value!");
            None
        }
        ClockStyleProto::AnalogRect => Some(ClockStyle::AnalogRect),
        ClockStyleProto::AnalogRound => Some(ClockStyle::AnalogRound),
        ClockStyleProto::Digital => Some(ClockStyle::Digital),
    };

    let maybe_numbers_font_style = match clock_proto.numbers_font_style() {
        FontStyleProto::Unspecified => {
            field_violations.push(format!("{field}.numbers_font_style"), "Missing value!");
            None
        }
        FontStyleProto::Light => Some(FontStyle::Light),
        FontStyleProto::Medium => Some(FontStyle::Medium),
        FontStyleProto::Bold => Some(FontStyle::Bold),
    };

    let maybe_timezone = clock_proto
        .timezone
        .map(|timezone| Timezone::from_str(&timezone))
        .transpose()
        .tap_err(|_| field_violations.push(format!("{field}.timezone"), "Invalid timezone!"))
        .ok();

    let maybe_kind = maybe_clock_style
        .zip(maybe_numbers_font_style)
        .zip(maybe_timezone)
        .map(|((clock_style, numbers_font_style), timezone)| {
            WidgetKind::Clock(ClockWidget {
                clock_style,
                numbers_font_style,
                show_date: clock_proto.show_date,
                show_seconds: clock_proto.show_seconds,
                show_timezone: clock_proto.show_timezone,
                timezone: if clock_proto.show_timezone {
                    timezone
                } else {
                    None
                },
            })
        });

    (maybe_kind, field_violations)
}

fn parse_ticker_btc_widget_kind(
    field: &str,
    ticker_btc_proto: web::TickerBtcWidget,
) -> ParseOutput<WidgetKind> {
    use web::ticker_btc_widget::TimeFrame as TimeFrameProto;

    let mut field_violations = FieldViolations::new();

    let maybe_time_frame = match ticker_btc_proto.time_frame() {
        TimeFrameProto::Unspecified => {
            field_violations.push(format!("{field}.time_frame"), "Missing value!");
            None
        }
        TimeFrameProto::Day1 => Some(TickerTimeFrame::Day1),
        TimeFrameProto::Week1 => Some(TickerTimeFrame::Week1),
        TimeFrameProto::Week2 => Some(TickerTimeFrame::Week2),
        TimeFrameProto::Month1 => Some(TickerTimeFrame::Month1),
        TimeFrameProto::Month3 => Some(TickerTimeFrame::Month3),
        TimeFrameProto::Month6 => Some(TickerTimeFrame::Month6),
        TimeFrameProto::Year1 => Some(TickerTimeFrame::Year1),
        TimeFrameProto::Year2 => Some(TickerTimeFrame::Year2),
        TimeFrameProto::Year5 => Some(TickerTimeFrame::Year5),
        TimeFrameProto::All => Some(TickerTimeFrame::All),
    };

    let maybe_kind =
        maybe_time_frame.map(|time_frame| WidgetKind::TickerBtc(TickerBtcWidget { time_frame }));

    (maybe_kind, field_violations)
}

fn parse_block_height_widget_kind(
    field: &str,
    block_height_proto: web::BlockHeightWidget,
) -> ParseOutput<WidgetKind> {
    use web::FontStyle as FontStyleProto;

    let mut field_violations = FieldViolations::new();

    let maybe_numbers_font_style = match block_height_proto.numbers_font_style() {
        FontStyleProto::Unspecified => {
            field_violations.push(format!("{field}.numbers_font_style"), "Missing value!");
            None
        }
        FontStyleProto::Light => Some(FontStyle::Light),
        FontStyleProto::Medium => Some(FontStyle::Medium),
        FontStyleProto::Bold => Some(FontStyle::Bold),
    };

    let maybe_kind = maybe_numbers_font_style.map(|numbers_font_style| {
        WidgetKind::BlockHeight(BlockHeightWidget {
            show_timestamp: block_height_proto.show_timestamp,
            numbers_font_style,
        })
    });

    (maybe_kind, field_violations)
}

fn parse_braiins_pool_widget_kind(
    field: &str,
    braiins_pool_proto: web::BraiinsPoolWidget,
) -> ParseOutput<WidgetKind> {
    use web::braiins_pool_widget::BraiinsPoolStyle as PoolStyleProto;
    use web::braiins_pool_widget::TimeFrame as TimeFrameProto;

    let mut field_violations = FieldViolations::new();

    let maybe_scene_style = match braiins_pool_proto.braiins_pool_style() {
        PoolStyleProto::Unspecified => {
            field_violations.push(format!("{field}.pool_style"), "Missing value!");
            None
        }
        PoolStyleProto::Overview => Some(PoolStyle::Overview),
        PoolStyleProto::Bigchart => Some(PoolStyle::BigChart),
    };

    let maybe_time_frame = match braiins_pool_proto.time_frame() {
        TimeFrameProto::Unspecified => {
            field_violations.push(format!("{field}.time_frame"), "Missing value!");
            None
        }
        TimeFrameProto::Hour4 => Some(PoolChartTimeFrame::Hours4),
        TimeFrameProto::Hour12 => Some(PoolChartTimeFrame::Hours12),
        TimeFrameProto::Hour24 => Some(PoolChartTimeFrame::Hours24),
        TimeFrameProto::Day7 => Some(PoolChartTimeFrame::Days7),
    };

    let maybe_account_id = braiins_pool_proto
        .account_id
        .map(|account_id| AccountId::from_str(&account_id))
        .transpose()
        .tap_err(|_| field_violations.push(format!("{field}.account_id"), "Missing value!"))
        .ok();

    let maybe_kind = maybe_scene_style
        .zip(maybe_time_frame)
        .zip(maybe_account_id)
        .map(|((pool_style, chart_frame), account_id)| {
            WidgetKind::BraiinsPool(BraiinsPoolWidget {
                pool_style,
                chart_frame,
                account_id,
            })
        });

    (maybe_kind, field_violations)
}

fn parse_remote_image_widget_kind(
    field: impl AsRef<str> + Display,
    remote_image_proto: web::RemoteImageWidget,
) -> ParseOutput<WidgetKind> {
    let mut field_violations = FieldViolations::new();

    let refresh_duration = Some(Duration::from_secs(
        remote_image_proto.refresh_duration_sec.into(),
    ))
    .filter(|duration| !duration.is_zero())
    .tap_none(|| {
        field_violations.push(
            format!("{field}.refresh_duration_sec"),
            "Refresh duration cannot be zero!",
        );
    });

    let maybe_kind = refresh_duration.map(|refresh_duration| {
        WidgetKind::RemoteImage(RemoteImageWidget {
            url: remote_image_proto.url,
            refresh_duration,
        })
    });

    (maybe_kind, field_violations)
}

fn parse_countdown_widget_kind(
    field: impl AsRef<str> + Display,
    countdown_proto: web::CountdownWidget,
) -> ParseOutput<WidgetKind> {
    use web::FontStyle as FontStyleProto;
    use web::LedEffect as LedEffectProto;

    let mut field_violations = FieldViolations::new();

    // Validate label is not empty
    let maybe_label = if countdown_proto.label.trim().is_empty() {
        field_violations.push(format!("{field}.label"), "Label cannot be empty!");
        None
    } else {
        Some(countdown_proto.label.clone())
    };

    // Validate target timestamp is positive
    let maybe_target_timestamp = countdown_proto
        .target_timestamp
        .map(|ts| ts.seconds)
        .filter(|&s| s > 0)
        .or_else(|| {
            field_violations.push(
                format!("{field}.target_timestamp"),
                "Target timestamp must be positive!",
            );
            None
        });

    // Parse font style
    let maybe_numbers_font_style = match countdown_proto.numbers_font_style() {
        FontStyleProto::Unspecified => {
            field_violations.push(format!("{field}.numbers_font_style"), "Missing value!");
            None
        }
        FontStyleProto::Light => Some(FontStyle::Light),
        FontStyleProto::Medium => Some(FontStyle::Medium),
        FontStyleProto::Bold => Some(FontStyle::Bold),
    };

    // Parse completion action (optional) with nested LedSettings/SoundSettings
    let completion_action = if let Some(action) = countdown_proto.completion_action {
        let led = action.led.map(|led_settings| {
            let effect = match led_settings.effect() {
                LedEffectProto::Unspecified | LedEffectProto::None => LedEffectKind::None,
                LedEffectProto::Solid => LedEffectKind::Solid,
                LedEffectProto::Breathe => LedEffectKind::Breathe,
                LedEffectProto::Chase => LedEffectKind::Chase,
                LedEffectProto::KnightRider => LedEffectKind::KnightRider,
                LedEffectProto::Scan => LedEffectKind::Scan,
                LedEffectProto::Snake => LedEffectKind::Snake,
            };

            let color = led_settings
                .color
                .map_or(Rgb { r: 0, g: 0, b: 0 }, |c| Rgb {
                    r: c.r.min(255) as u8,
                    g: c.g.min(255) as u8,
                    b: c.b.min(255) as u8,
                });

            LedSettings { effect, color }
        });

        let sound = if let Some(sound_settings) = action.sound {
            if let Ok(sound) = Sounds::from_str(&sound_settings.sound_id) {
                Some(SoundSettings {
                    sound,
                    volume: sound_settings.volume.min(100) as u8,
                })
            } else {
                field_violations.push(
                    format!("{field}.completion_action.sound.sound_id"),
                    format!("Unknown sound ID: {}", sound_settings.sound_id),
                );
                None
            }
        } else {
            None
        };

        Some(CountdownCompletionAction { led, sound })
    } else {
        None
    };

    // Serialize completion_action to JSON for storage in bmc-display's CountdownWidget
    let completion_action_json =
        completion_action
            .as_ref()
            .and_then(|action| match serde_json::to_value(action) {
                Ok(v) => Some(v),
                Err(err) => {
                    warn!(?err, "Failed to serialize completion action");
                    None
                }
            });

    let maybe_kind = maybe_label
        .zip(maybe_target_timestamp)
        .zip(maybe_numbers_font_style)
        .map(|((label, target_timestamp), numbers_font_style)| {
            WidgetKind::Countdown(CountdownWidget {
                label,
                target_timestamp,
                background_color: countdown_proto.background_color,
                numbers_font_style,
                completion_action: completion_action_json,
            })
        });

    (maybe_kind, field_violations)
}

fn map_proto_to_remote_widget_kind(remote_widget_proto: web::RemoteWidget) -> WidgetKind {
    WidgetKind::RemoteWidget(RemoteWidget {
        name: remote_widget_proto.name,
        description: remote_widget_proto.description,
        widget_url: remote_widget_proto.widget_url,
        icon_url: remote_widget_proto.icon_url,
        params: map_protobuf_struct_to_params(remote_widget_proto.params.unwrap_or_default()),
    })
}

fn map_scene_to_proto(scene: Scene) -> web::Scene {
    let kind = match &scene.kind {
        SceneKind::Fullscreen => web::scene::Kind::Fullscreen(web::scene::Fullscreen {
            widget: {
                let widget = scene.widgets.into_values().next().map(map_widget_to_proto);
                debug_assert!(widget.is_some());
                widget
            },
        }),
        SceneKind::Combined => web::scene::Kind::Combined(web::scene::Combined {
            widgets: scene
                .widgets
                .into_values()
                .map(map_widget_to_proto)
                .collect(),
        }),
    };

    let cycle_duration_sec = scene.cycle_duration.map(|cycle_duration| {
        #[expect(clippy::cast_possible_truncation)]
        let cycle_duration_sec = cycle_duration.as_secs() as u32;
        cycle_duration_sec
    });

    web::Scene {
        id: scene.id.to_string(),
        enabled: scene.enabled,
        cycle_duration_sec,
        kind: Some(kind),
    }
}

fn map_widget_to_proto(widget: Widget) -> web::Widget {
    let kind = match widget.kind {
        WidgetKind::Clock(clock) => map_clock_to_proto(clock),
        WidgetKind::TickerBtc(ticker_btc) => map_ticker_btc_to_proto(&ticker_btc),
        WidgetKind::BlockHeight(block_height) => map_block_height_to_proto(&block_height),
        WidgetKind::BraiinsPool(braiins_pool) => map_braiins_pool_to_proto(braiins_pool),
        WidgetKind::RemoteImage(remote_image) => map_remote_image_to_proto(remote_image),
        WidgetKind::BlockchainData => map_blockchain_data_to_proto(),
        WidgetKind::RemoteWidget(remote_widget) => map_remote_widget_to_proto(remote_widget),
        WidgetKind::HalvingCountdown => map_halving_countdown_to_proto(),
        WidgetKind::Countdown(countdown) => map_countdown_to_proto(countdown),
    };

    web::Widget {
        id: widget.id.to_string(),
        position: Some(web::WidgetPosition {
            row: widget.position.row.into(),
            col: widget.position.col.into(),
        }),
        size: match widget.size {
            WidgetSize::Small => web::WidgetSize::Small.into(),
            WidgetSize::Medium => web::WidgetSize::Medium.into(),
            WidgetSize::Large => web::WidgetSize::Large.into(),
            WidgetSize::Full => web::WidgetSize::Full.into(),
        },
        kind: Some(kind),
    }
}

fn map_clock_to_proto(clock: ClockWidget) -> web::WidgetKind {
    use web::FontStyle as FontStyleProto;
    use web::clock_widget::ClockStyle as ClockStyleProto;

    let proto = web::ClockWidget {
        clock_style: match clock.clock_style {
            ClockStyle::AnalogRound => ClockStyleProto::AnalogRound,
            ClockStyle::AnalogRect => ClockStyleProto::AnalogRect,
            ClockStyle::Digital => ClockStyleProto::Digital,
        }
        .into(),
        numbers_font_style: match clock.numbers_font_style {
            FontStyle::Light => FontStyleProto::Light,
            FontStyle::Medium => FontStyleProto::Medium,
            FontStyle::Bold => FontStyleProto::Bold,
        }
        .into(),
        show_date: clock.show_date,
        show_seconds: clock.show_seconds,
        show_timezone: clock.show_timezone,
        timezone: clock.timezone.map(|timezone| timezone.iana().to_owned()),
    };

    web::WidgetKind {
        value: Some(web::widget_kind::Value::Clock(proto)),
    }
}

fn parse_scene_cycling(
    field: impl AsRef<str> + Display,
    input: Option<web::SceneCycling>,
) -> ParseOutput<SceneCycling> {
    use web::SceneCyclingTransition as SceneCyclingTransitionProto;

    let mut all_field_violations = FieldViolations::new();

    let Some(input) = input else {
        all_field_violations.push(field.as_ref(), "Missing value!");
        return (None, all_field_violations);
    };

    let (maybe_automatic_cycling_default_duration, field_violations) = parse_scene_cycle_duration(
        format!("{field}.automatic_cycling_default_duration_sec"),
        Some(input.automatic_cycling_default_duration_sec),
    );
    all_field_violations.extend(field_violations);

    let maybe_transition = match input.transition() {
        SceneCyclingTransitionProto::Unspecified => {
            all_field_violations.push(format!("{field}.transition"), "Missing value!");
            None
        }
        SceneCyclingTransitionProto::Slide => Some(SceneCyclingTransition::Slide),
        SceneCyclingTransitionProto::Fade => Some(SceneCyclingTransition::Fade),
    };

    let maybe_scene_cycling = maybe_automatic_cycling_default_duration
        .zip(maybe_transition)
        .map(
            |(automatic_cycling_default_duration, transition)| SceneCycling {
                automatic_cycling_enabled: input.automatic_cycling_enabled,
                automatic_cycling_default_duration: automatic_cycling_default_duration
                    .expect("BUG: automatic_cycling_default_duration should be present, since we wrapped input into Some"),
                transition,
            },
        );

    (maybe_scene_cycling, all_field_violations)
}

#[expect(clippy::needless_pass_by_value)]
fn map_scene_cycling_to_proto(config: SceneCycling) -> web::SceneCycling {
    #[expect(clippy::cast_possible_truncation)]
    let automatic_cycling_default_duration_sec =
        config.automatic_cycling_default_duration.as_secs() as u32;

    web::SceneCycling {
        automatic_cycling_enabled: config.automatic_cycling_enabled,
        automatic_cycling_default_duration_sec,
        transition: match config.transition {
            SceneCyclingTransition::Slide => web::SceneCyclingTransition::Slide.into(),
            SceneCyclingTransition::Fade => web::SceneCyclingTransition::Fade.into(),
        },
    }
}

fn map_ticker_btc_to_proto(ticker_btc: &TickerBtcWidget) -> web::WidgetKind {
    use web::ticker_btc_widget::TimeFrame as TimeFrameProto;

    let proto = web::TickerBtcWidget {
        time_frame: match ticker_btc.time_frame {
            TickerTimeFrame::Day1 => TimeFrameProto::Day1,
            TickerTimeFrame::Week1 => TimeFrameProto::Week1,
            TickerTimeFrame::Week2 => TimeFrameProto::Week2,
            TickerTimeFrame::Month1 => TimeFrameProto::Month1,
            TickerTimeFrame::Month3 => TimeFrameProto::Month3,
            TickerTimeFrame::Month6 => TimeFrameProto::Month6,
            TickerTimeFrame::Year1 => TimeFrameProto::Year1,
            TickerTimeFrame::Year2 => TimeFrameProto::Year2,
            TickerTimeFrame::Year5 => TimeFrameProto::Year5,
            TickerTimeFrame::All => TimeFrameProto::All,
        }
        .into(),
    };

    web::WidgetKind {
        value: Some(web::widget_kind::Value::TickerBtc(proto)),
    }
}

fn map_block_height_to_proto(block_height: &BlockHeightWidget) -> web::WidgetKind {
    use web::FontStyle as FontStyleProto;

    let proto = web::BlockHeightWidget {
        show_timestamp: block_height.show_timestamp,
        numbers_font_style: match block_height.numbers_font_style {
            FontStyle::Light => FontStyleProto::Light,
            FontStyle::Medium => FontStyleProto::Medium,
            FontStyle::Bold => FontStyleProto::Bold,
        }
        .into(),
    };

    web::WidgetKind {
        value: Some(web::widget_kind::Value::BlockHeight(proto)),
    }
}

fn map_braiins_pool_to_proto(braiins_pool: BraiinsPoolWidget) -> web::WidgetKind {
    use web::braiins_pool_widget::BraiinsPoolStyle as PoolStyleProto;
    use web::braiins_pool_widget::TimeFrame as TimeFrameProto;

    let proto = web::BraiinsPoolWidget {
        braiins_pool_style: match braiins_pool.pool_style {
            PoolStyle::Overview => PoolStyleProto::Overview,
            PoolStyle::BigChart => PoolStyleProto::Bigchart,
        }
        .into(),
        time_frame: match braiins_pool.chart_frame {
            PoolChartTimeFrame::Hours4 => TimeFrameProto::Hour4,
            PoolChartTimeFrame::Hours12 => TimeFrameProto::Hour12,
            PoolChartTimeFrame::Hours24 => TimeFrameProto::Hour24,
            PoolChartTimeFrame::Days7 => TimeFrameProto::Day7,
        }
        .into(),
        account_id: braiins_pool
            .account_id
            .map(|account_id| account_id.to_string()),
    };

    web::WidgetKind {
        value: Some(web::widget_kind::Value::BraiinsPool(proto)),
    }
}

fn map_remote_image_to_proto(remote_image: RemoteImageWidget) -> web::WidgetKind {
    let proto = web::RemoteImageWidget {
        url: remote_image.url,
        refresh_duration_sec: {
            #[expect(clippy::cast_possible_truncation)]
            let refresh_duration_sec = remote_image.refresh_duration.as_secs() as u32;

            refresh_duration_sec
        },
    };

    web::WidgetKind {
        value: Some(web::widget_kind::Value::RemoteImage(proto)),
    }
}

fn map_remote_widget_to_proto(remote_widget: RemoteWidget) -> web::WidgetKind {
    let proto = web::RemoteWidget {
        name: remote_widget.name,
        description: remote_widget.description,
        widget_url: remote_widget.widget_url,
        icon_url: remote_widget.icon_url,
        params: Some(map_params_to_protobuf_struct(remote_widget.params)),
    };

    web::WidgetKind {
        value: Some(web::widget_kind::Value::RemoteWidget(proto)),
    }
}

fn map_remote_widget_to_proto_remote_widget(remote_widget: RemoteWidget) -> web::RemoteWidget {
    web::RemoteWidget {
        name: remote_widget.name,
        description: remote_widget.description,
        widget_url: remote_widget.widget_url,
        icon_url: remote_widget.icon_url,
        params: Some(map_params_to_protobuf_struct(remote_widget.params)),
    }
}

fn map_blockchain_data_to_proto() -> web::WidgetKind {
    web::WidgetKind {
        value: Some(web::widget_kind::Value::BlockchainData(
            web::BlockchainDataWidget {},
        )),
    }
}

fn map_halving_countdown_to_proto() -> web::WidgetKind {
    web::WidgetKind {
        value: Some(web::widget_kind::Value::HalvingCountdown(
            web::HalvingCountdownWidget {},
        )),
    }
}

fn map_countdown_to_proto(countdown: CountdownWidget) -> web::WidgetKind {
    use web::FontStyle as FontStyleProto;
    use web::LedEffect as LedEffectProto;

    let completion_action = countdown
        .completion_action
        .and_then(|json| {
            serde_json::from_value::<CountdownCompletionAction>(json)
                .tap_err(|err| warn!(?err, "Failed to deserialize completion action"))
                .ok()
        })
        .map(|action| {
            let led = action.led.map(|led| {
                let effect = match led.effect {
                    LedEffectKind::None => LedEffectProto::None,
                    LedEffectKind::Solid => LedEffectProto::Solid,
                    LedEffectKind::Breathe => LedEffectProto::Breathe,
                    LedEffectKind::Chase => LedEffectProto::Chase,
                    LedEffectKind::KnightRider => LedEffectProto::KnightRider,
                    LedEffectKind::Scan => LedEffectProto::Scan,
                    LedEffectKind::Snake => LedEffectProto::Snake,
                };

                web::LedSettings {
                    effect: effect.into(),
                    color: Some(web::RgbColor {
                        r: led.color.r.into(),
                        g: led.color.g.into(),
                        b: led.color.b.into(),
                    }),
                }
            });

            let sound = action.sound.map(|sound| web::SoundSettings {
                sound_id: sound.sound.to_string(),
                volume: sound.volume.into(),
            });

            web::CountdownCompletionAction { led, sound }
        });

    let proto = web::CountdownWidget {
        label: countdown.label,
        target_timestamp: Some(prost_types::Timestamp {
            seconds: countdown.target_timestamp,
            nanos: 0,
        }),
        background_color: countdown.background_color,
        numbers_font_style: match countdown.numbers_font_style {
            FontStyle::Light => FontStyleProto::Light,
            FontStyle::Medium => FontStyleProto::Medium,
            FontStyle::Bold => FontStyleProto::Bold,
        }
        .into(),
        completion_action,
    };

    web::WidgetKind {
        value: Some(web::widget_kind::Value::Countdown(proto)),
    }
}

async fn remote_widget_url_validation(
    remote_widget: &web::RemoteWidget,
    field_violations: &mut FieldViolations,
) -> Option<WidgetKind> {
    let url = remote_widget.widget_url.clone();
    let mut parsed_url = match Url::parse(&url) {
        Ok(url) => url,
        Err(err) => {
            warn!(?err, "Invalid URL, stopping");
            field_violations.push("widget_kind.remote_widget", "Invalid URL!");
            return None;
        }
    };
    match parsed_url.path_segments_mut() {
        Ok(mut path_segments) => path_segments.push("metadata"),
        Err(err) => {
            warn!(?err, "Failed to get path segments");
            field_violations.push("widget_kind.remote_widget", "Unexpected error!");
            return None;
        }
    };

    let client = match Client::builder().timeout(REQUEST_TIMEOUT).build() {
        Ok(client) => client,
        Err(err) => {
            warn!(?err, "Failed to create reqwest client, stopping");
            field_violations.push("widget_kind.remote_widget", "Unexpected error!");
            return None;
        }
    };

    let metadata_response = match client.get(parsed_url.clone()).send().await {
        Ok(response) => {
            if !response.status().is_success() {
                warn!("Failed to get metadata, stopping");
                field_violations.push("widget_kind.remote_widget", "Unexpected error!");
                return None;
            }
            response
        }
        Err(err) => {
            warn!(?err, "Failed to get metadata, stopping");
            field_violations.push("widget_kind.remote_widget", "Unexpected error!");
            return None;
        }
    };

    let metadata = match metadata_response.json::<RemoteWidgetMetadata>().await {
        Ok(metadata) => metadata,
        Err(err) => {
            warn!(?err, "Failed to parse metadata, stopping");
            field_violations.push("widget_kind.remote_widget", "Unexpected error!");
            return None;
        }
    };

    let icon_url = match parsed_url.join(&metadata.assets.icon) {
        Ok(url) => url,
        Err(err) => {
            warn!(?err, "Failed to add path segments, stopping");
            field_violations.push("widget_kind.remote_widget", "Unexpected error!");
            return None;
        }
    };

    Some(WidgetKind::RemoteWidget(RemoteWidget {
        name: metadata.name,
        description: metadata.description,
        widget_url: url,
        icon_url: icon_url.to_string(),
        //NOTE: We will serialize only user defined params. No params mean default values.
        ..Default::default()
    }))
}

fn json_to_proto_value(v: JsonValue) -> ProstValue {
    match v {
        JsonValue::Null => ProstValue {
            kind: Some(ProstKind::NullValue(0)),
        },
        JsonValue::Bool(b) => ProstValue {
            kind: Some(ProstKind::BoolValue(b)),
        },
        JsonValue::Number(n) => ProstValue {
            kind: Some(ProstKind::NumberValue(n.as_f64().unwrap_or_default())),
        },
        JsonValue::String(s) => ProstValue {
            kind: Some(ProstKind::StringValue(s)),
        },
        JsonValue::Array(arr) => {
            let values = arr.into_iter().map(json_to_proto_value).collect();
            ProstValue {
                kind: Some(ProstKind::ListValue(ListValue { values })),
            }
        }
        JsonValue::Object(map) => {
            let fields = map
                .into_iter()
                .map(|(k, v)| (k, json_to_proto_value(v)))
                .collect();
            ProstValue {
                kind: Some(ProstKind::StructValue(Struct { fields })),
            }
        }
    }
}

fn proto_to_json_value(v: ProstValue) -> JsonValue {
    match v.kind {
        None | Some(ProstKind::NullValue(_)) => JsonValue::Null,
        Some(ProstKind::NumberValue(n)) => {
            JsonValue::Number(serde_json::Number::from_f64(n).unwrap_or_else(|| 0.into()))
        }
        Some(ProstKind::StringValue(s)) => JsonValue::String(s),
        Some(ProstKind::BoolValue(b)) => JsonValue::Bool(b),
        Some(ProstKind::ListValue(list)) => {
            JsonValue::Array(list.values.into_iter().map(proto_to_json_value).collect())
        }
        Some(ProstKind::StructValue(s)) => {
            let mut map = JsonValue::Object(serde_json::Map::default());
            if let JsonValue::Object(ref mut inner) = map {
                for (k, v) in s.fields {
                    inner.insert(k, proto_to_json_value(v));
                }
            }
            map
        }
    }
}

fn map_protobuf_struct_to_params(param: Struct) -> JsonValue {
    let mut map = serde_json::Map::new();

    for (k, v) in param.fields {
        map.insert(k, proto_to_json_value(v));
    }

    JsonValue::Object(map)
}

fn map_params_to_protobuf_struct(param: JsonValue) -> Struct {
    if let JsonValue::Object(map) = param {
        Struct {
            fields: map
                .into_iter()
                .map(|(k, v)| (k, json_to_proto_value(v)))
                .collect(),
        }
    // Parameters in the JSON are key value pairs.
    // We expect only the Object variant.
    } else {
        Struct {
            fields: std::collections::BTreeMap::default(),
        }
    }
}
