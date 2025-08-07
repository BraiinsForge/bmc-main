// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::config::ConfigHandle;
use crate::web::grpc::GrpcError;
use crate::widget_tasks::WidgetTasks;
use bmc_display::data::{
    AddWidgetError, ClockStyle, ClockWidget, FontStyle, RemoveWidgetError, Scene, SceneId,
    SceneKind, UpdateWidgetError, Widget, WidgetId, WidgetKind, WidgetPosition, WidgetSize,
};
use bmc_display::display_controller::DisplayController;
use bmc_grpc::web;
use bmc_shared_time::time::Timezone;
use futures::StreamExt;
use futures::stream::BoxStream;
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
use tracing::error;

pub(crate) struct SceneManagementService {
    config_handle: Arc<RwLock<ConfigHandle>>,
    display_controller: DisplayController,
    widget_tasks: WidgetTasks,
    preview_scene_id: Arc<Mutex<Option<SceneId>>>,
}

impl SceneManagementService {
    pub(crate) fn new(
        config_handle: Arc<RwLock<ConfigHandle>>,
        display_controller: DisplayController,
        widget_tasks: WidgetTasks,
    ) -> Self {
        Self {
            config_handle,
            display_controller,
            widget_tasks,
            preview_scene_id: Arc::default(),
        }
    }
}

#[async_trait::async_trait]
impl web::scene_management_service_server::SceneManagementService for SceneManagementService {
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
            parse_widget_kind_with_default_params("widget_kind", request.widget_kind);

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
                let mut config = config_handle.write().await;
                let mut temp_config = config.clone();

                let new_scene = Scene::fullscreen(widget_kind);
                let new_scene_id = new_scene.id.clone();

                let replaced_scene = temp_config
                    .scenes
                    .insert(new_scene_id.clone(), new_scene.clone());
                debug_assert!(replaced_scene.is_none());

                if let Err(err) = temp_config.sync_to_storage().await {
                    error!("Cannot save config: {}", err);
                    return Err(Status::internal("Failed to save configuration"));
                }
                *config = temp_config;

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
                let mut config = config_handle.write().await;
                let mut temp_config = config.clone();

                let new_scene = Scene::combined();
                let new_scene_id = new_scene.id.clone();

                let replaced_scene = temp_config
                    .scenes
                    .insert(new_scene_id.clone(), new_scene.clone());
                debug_assert!(replaced_scene.is_none());

                if let Err(err) = temp_config.sync_to_storage().await {
                    error!("Cannot save config: {}", err);
                    return Err(Status::internal("Failed to save configuration"));
                }
                *config = temp_config;

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
                let mut config = config_handle.write().await;
                let mut temp_config = config.clone();

                let scene = temp_config
                    .scenes
                    .get_mut(&id)
                    .ok_or_else(|| Status::not_found("Scene not found"))?;

                let previously_enabled = scene.enabled;

                scene.enabled = enabled;
                scene.cycle_duration = cycle_duration;

                if let Err(err) = temp_config.sync_to_storage().await {
                    error!("Cannot save config: {}", err);
                    return Err(Status::internal("Failed to save configuration"));
                }
                *config = temp_config;

                if enabled != previously_enabled {
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

                if let Err(err) = temp_config.sync_to_storage().await {
                    error!("Cannot save config: {}", err);
                    return Err(Status::internal("Failed to save configuration"));
                }
                *config = temp_config;

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
                    cloned_scene_id.clone(),
                    cloned_scene.clone(),
                );
                debug_assert!(replaced_scene.is_none());

                if let Err(err) = temp_config.sync_to_storage().await {
                    error!("Cannot save config: {}", err);
                    return Err(Status::internal("Failed to save configuration"));
                }
                *config = temp_config;

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
                let mut config = config_handle.write().await;
                let mut temp_config = config.clone();

                let scene = temp_config
                    .scenes
                    .shift_remove(&id)
                    .ok_or_else(|| Status::not_found("Scene not found"))?;

                if let Err(err) = temp_config.sync_to_storage().await {
                    error!("Cannot save config: {}", err);
                    return Err(Status::internal("Failed to save configuration"));
                }
                *config = temp_config;

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

        struct DisableScenePreviewOnDrop {
            display_controller: DisplayController,
            widget_tasks: WidgetTasks,
            preview_scene_id: Arc<Mutex<Option<SceneId>>>,
            started_temporary_widget_tasks: bool,
        }

        impl Drop for DisableScenePreviewOnDrop {
            fn drop(&mut self) {
                self.display_controller.set_preview_scene(None);

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

        let (kind, field_violations) = parse_widget_kind_with_default_params("kind", request.kind);
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

                let new_widget_id = new_widget.id.clone();

                if let Err(err) = temp_config.sync_to_storage().await {
                    error!("Cannot save display config: {}", err);
                    return Err(Status::internal("Failed to save widget configuration"));
                }
                *config = temp_config;

                let scene = &config.scenes[&scene_id];
                let is_preview_scene = preview_scene_id
                    .lock()
                    .await
                    .as_ref()
                    .is_some_and(|preview_scene_id| *preview_scene_id == scene_id);

                if scene.enabled || is_preview_scene {
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

                if let Err(err) = temp_config.sync_to_storage().await {
                    error!("Cannot save display config: {}", err);
                    return Err(Status::internal("Failed to save widget configuration"));
                }
                *config = temp_config;

                let scene = &config.scenes[&scene_id];
                let is_preview_scene = preview_scene_id
                    .lock()
                    .await
                    .as_ref()
                    .is_some_and(|preview_scene_id| *preview_scene_id == scene_id);

                if scene.enabled || is_preview_scene {
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

                if let Err(err) = temp_config.sync_to_storage().await {
                    error!("Cannot save display config: {}", err);
                    return Err(Status::internal("Failed to save widget configuration"));
                }
                *config = temp_config;

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

    pub fn push(&mut self, field: impl Into<String>, description: impl Into<String>) {
        self.0.push(FieldViolation::new(field, description));
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

fn parse_scene_id(field: &str, input: &str) -> ParseOutput<SceneId> {
    let mut field_violations = FieldViolations::new();

    let maybe_id = SceneId::from_str(input).ok().tap_none(|| {
        field_violations.push(field, "Invalid scene ID!");
    });

    (maybe_id, field_violations)
}

fn parse_widget_id(field: &str, input: &str) -> ParseOutput<WidgetId> {
    let mut field_violations = FieldViolations::new();

    let maybe_id = WidgetId::from_str(input).ok().tap_none(|| {
        field_violations.push(field, "Invalid widget ID!");
    });

    (maybe_id, field_violations)
}

fn parse_scene_cycle_duration(field: &str, input: u32) -> ParseOutput<Duration> {
    let mut field_violations = FieldViolations::new();

    let cycle_duration = Duration::from_secs(input.into());

    if cycle_duration < Scene::MIN_CYCLE_DURATION {
        field_violations.push(
            field,
            format!("Out of range: {}..", Scene::MIN_CYCLE_DURATION.as_secs()),
        );
        (None, field_violations)
    } else {
        (Some(cycle_duration), field_violations)
    }
}

fn parse_widget_position(
    field: &str,
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

fn parse_widget_size(field: &str, input: web::WidgetSize) -> ParseOutput<WidgetSize> {
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

fn parse_widget_kind_with_default_params(
    field: &str,
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
    };

    (Some(kind), field_violations)
}

fn parse_widget_kind(field: &str, input: Option<web::WidgetKind>) -> ParseOutput<WidgetKind> {
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
            let (maybe_kind, field_violations) = parse_clock_widget_kind("clock", clock_proto);
            all_field_violations.extend(field_violations);
            maybe_kind
        }
    };

    (maybe_kind, all_field_violations)
}

fn parse_clock_widget_kind(field: &str, clock_proto: web::ClockWidget) -> ParseOutput<WidgetKind> {
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
                timezone,
            })
        });

    (maybe_kind, field_violations)
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

    #[expect(clippy::cast_possible_truncation)]
    let cycle_duration_sec = scene.cycle_duration.as_secs() as u32;

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
        timezone: clock.timezone.map(|timezone| timezone.to_string()),
    };

    web::WidgetKind {
        value: Some(web::widget_kind::Value::Clock(proto)),
    }
}
