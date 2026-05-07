// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::Arc;
use std::time::Duration;

use bmc_grpc::web;
use bmc_grpc::web::scene_management_service_server::SceneManagementService as GrpcSceneManagementService;
use bmc_ipc::SizeType;
use bmc_widget::ParamDefinition;
use futures::stream::{BoxStream, StreamExt};
use tokio::sync::{Mutex, RwLock};
use tokio::time;
use tokio_stream::wrappers::IntervalStream;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::config::ConfigHandle;
use crate::data::{SceneCycling, SceneCyclingTransition};
use crate::scene;
use crate::widget::{Coordinator, WidgetRegistry};

pub(crate) struct SceneManagementService {
    widget_registry: Arc<WidgetRegistry>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    coordinator: Arc<Coordinator>,
    /// Scene currently held open by a `preview_scene` stream. While set,
    /// that scene overrides the first-enabled pick in `restore_active_scene`
    /// so edits made during a preview stay focused on it.
    preview_scene_id: Arc<Mutex<Option<scene::SceneId>>>,
}

impl SceneManagementService {
    pub(crate) fn new(
        widget_registry: Arc<WidgetRegistry>,
        config_handle: Arc<RwLock<ConfigHandle>>,
        coordinator: Arc<Coordinator>,
    ) -> Self {
        Self {
            widget_registry,
            config_handle,
            coordinator,
            preview_scene_id: Arc::default(),
        }
    }

    /// Refresh the compositor's cycling list from current config; if a
    /// preview is active, also push that scene as the destructive active.
    async fn restore_active_scene(&self) {
        let config = self.config_handle.read().await;
        self.coordinator.refresh_scene_cycling(&config.scenes);
        let preview_id = *self.preview_scene_id.lock().await;
        if let Some(preview_id) = preview_id
            && let Some(scene) = config.scenes.get(&preview_id)
        {
            self.coordinator.set_active_scene(scene);
        }
    }

    /// Whether the given scene is currently shown on the display — either
    /// because it's enabled or because it's being previewed. Gates the
    /// "apply to live compositor" branches of add_widget / update_widget so
    /// edits during a preview reach the widget process that's already up.
    async fn scene_is_showing(&self, scene_id: &scene::SceneId) -> bool {
        let config = self.config_handle.read().await;
        let enabled = config.scenes.get(scene_id).is_some_and(|s| s.enabled);
        drop(config);
        enabled || self.preview_scene_id.lock().await.as_ref() == Some(scene_id)
    }

    /// Spawn a widget and log failures instead of propagating them.
    async fn try_spawn_widget(&self, scene_id: &scene::SceneId, widget: &scene::Widget) {
        // Settings travel through the compositor's cached state — each
        // widget's initial configure batch replays them on connect, so the
        // runtime spawn path no longer threads them through.
        self.coordinator.spawn_widget(scene_id, widget).await;
    }

    /// Save config, returning a gRPC-friendly error on failure.
    async fn save_config(config: &mut ConfigHandle) -> Result<(), Status> {
        config
            .save()
            .await
            .map_err(|e| Status::internal(format!("failed to save config: {e}")))
    }
}

fn build_widget_params(
    manifest_params: &std::collections::HashMap<String, ParamDefinition>,
    user_overrides: Option<&web::WidgetDataStruct>,
) -> serde_json::Value {
    let _ = manifest_params;
    let _ = user_overrides;
    unimplemented!("filled in by Phase 3")
}

fn size_type_to_proto(size: SizeType) -> i32 {
    match size {
        SizeType::Small => web::WidgetSize::Small.into(),
        SizeType::Medium => web::WidgetSize::Medium.into(),
        SizeType::Large => web::WidgetSize::Large.into(),
        SizeType::Full => web::WidgetSize::Full.into(),
    }
}

fn param_definition_to_proto(key: &str, param: &ParamDefinition) -> web::ManifestParamDefinition {
    let _ = key;
    let _ = param;
    unimplemented!("filled in by Phase 3")
}

fn widget_info_to_proto(info: &crate::widget::WidgetInfo) -> web::WidgetManifest {
    let manifest = &info.manifest;
    web::WidgetManifest {
        uid: manifest.uid.to_string(),
        name: manifest.name.clone(),
        description: manifest.description.clone(),
        version: manifest.version.to_string(),
        author_name: manifest.author.as_ref().map(|a| a.name.clone()),
        author_url: manifest.author.as_ref().and_then(|a| a.url.clone()),
        supported_sizes: manifest
            .sizes
            .iter()
            .copied()
            .map(size_type_to_proto)
            .collect(),
        params: manifest
            .params
            .iter()
            .map(|(key, param)| param_definition_to_proto(key, param))
            .collect(),
    }
}

fn proto_size_to_scene(size: i32) -> Result<scene::WidgetSize, Status> {
    match web::WidgetSize::try_from(size) {
        Ok(web::WidgetSize::Small) => Ok(scene::WidgetSize::Small),
        Ok(web::WidgetSize::Medium) => Ok(scene::WidgetSize::Medium),
        Ok(web::WidgetSize::Large) => Ok(scene::WidgetSize::Large),
        Ok(web::WidgetSize::Full) => Ok(scene::WidgetSize::Full),
        Ok(web::WidgetSize::Unspecified) => {
            Err(Status::invalid_argument("widget size unspecified"))
        }
        Err(_) => Err(Status::invalid_argument("invalid widget size")),
    }
}

/// Convert the proto's u32 row/col into the scene model's u8, rejecting
/// values that don't fit instead of silently clamping to u8::MAX — a
/// clamped position would subsequently fail the bounds check anyway,
/// but reporting it as an explicit argument error gives the client a
/// much more useful message than "out of grid bounds".
fn proto_position_to_scene(position: web::WidgetPosition) -> Result<scene::WidgetPosition, Status> {
    let row = u8::try_from(position.row).map_err(|_| {
        Status::invalid_argument(format!("row {} does not fit in u8", position.row))
    })?;
    let col = u8::try_from(position.col).map_err(|_| {
        Status::invalid_argument(format!("col {} does not fit in u8", position.col))
    })?;
    Ok(scene::WidgetPosition { row, col })
}

/// Rejects widgets that either fall outside the display grid or collide with
/// another widget in the same scene. `exclude_id` is set to the widget's own
/// id when validating an update so it doesn't report overlapping with itself.
fn validate_widget_placement(
    scene: &scene::Scene,
    widget: &scene::Widget,
    exclude_id: Option<scene::WidgetId>,
) -> Result<(), Status> {
    if !widget.in_bounds() {
        return Err(Status::invalid_argument(format!(
            "widget at ({},{}) size {} is out of grid bounds",
            widget.position.row, widget.position.col, widget.size,
        )));
    }

    let overlaps = scene
        .widgets
        .iter()
        .any(|(id, existing)| exclude_id != Some(*id) && existing.overlaps(widget));
    if overlaps {
        return Err(Status::invalid_argument(
            "widget overlaps with an existing widget",
        ));
    }

    Ok(())
}

fn reject_remove_widget_in_fullscreen(kind: &scene::SceneKind) -> Result<(), Status> {
    if *kind == scene::SceneKind::Fullscreen {
        return Err(Status::failed_precondition(
            "cannot remove widget in fullscreen scene",
        ));
    }
    Ok(())
}

fn reject_update_widget_in_fullscreen(
    kind: &scene::SceneKind,
    new_position: web::WidgetPosition,
    new_size: scene::WidgetSize,
) -> Result<(), Status> {
    if *kind != scene::SceneKind::Fullscreen {
        return Ok(());
    }
    if new_position.row != 0 || new_position.col != 0 {
        return Err(Status::failed_precondition(
            "cannot move widget in fullscreen scene",
        ));
    }
    if new_size != scene::WidgetSize::Full {
        return Err(Status::failed_precondition(
            "cannot resize widget in fullscreen scene",
        ));
    }
    Ok(())
}

fn scene_widget_size_to_proto(size: scene::WidgetSize) -> i32 {
    match size {
        scene::WidgetSize::Small => web::WidgetSize::Small.into(),
        scene::WidgetSize::Medium => web::WidgetSize::Medium.into(),
        scene::WidgetSize::Large => web::WidgetSize::Large.into(),
        scene::WidgetSize::Full => web::WidgetSize::Full.into(),
    }
}

fn scene_widget_to_proto(widget: &scene::Widget) -> web::Widget {
    let _ = widget;
    unimplemented!("filled in by Phase 3")
}

fn scene_to_proto(scene: &scene::Scene) -> web::Scene {
    let widgets: Vec<web::Widget> = scene.widgets.values().map(scene_widget_to_proto).collect();

    let kind = match scene.kind {
        scene::SceneKind::Fullscreen => web::scene::Kind::Fullscreen(web::scene::Fullscreen {
            widget: widgets.into_iter().next(),
        }),
        scene::SceneKind::Combined => web::scene::Kind::Combined(web::scene::Combined { widgets }),
    };

    web::Scene {
        id: scene.id.to_string(),
        enabled: scene.enabled,
        cycle_duration_sec: scene.cycle_duration.map(|d| {
            #[expect(clippy::cast_possible_truncation)]
            let secs = d.as_secs() as u32;
            secs
        }),
        kind: Some(kind),
    }
}

#[async_trait::async_trait]
impl GrpcSceneManagementService for SceneManagementService {
    async fn get_available_widgets(
        &self,
        _request: Request<()>,
    ) -> Result<Response<web::GetAvailableWidgetsResponse>, Status> {
        let widgets = self
            .widget_registry
            .list()
            .map(widget_info_to_proto)
            .collect();

        Ok(Response::new(web::GetAvailableWidgetsResponse { widgets }))
    }

    async fn get_widget_manifest(
        &self,
        request: Request<String>,
    ) -> Result<Response<web::WidgetManifest>, Status> {
        let uid_str = request.into_inner();
        let uid = Uuid::parse_str(&uid_str)
            .map_err(|_| Status::invalid_argument(format!("invalid widget UID: {uid_str}")))?;

        let info = self
            .widget_registry
            .get(&uid)
            .ok_or_else(|| Status::not_found(format!("widget not found: {uid}")))?;

        Ok(Response::new(widget_info_to_proto(info)))
    }

    // ── Scene read RPCs (from config) ──────────────────────────────────

    async fn get_scenes(
        &self,
        _request: Request<()>,
    ) -> Result<Response<web::GetScenesResponse>, Status> {
        let config = self.config_handle.read().await;
        let scenes = config.scenes.values().map(scene_to_proto).collect();
        Ok(Response::new(web::GetScenesResponse { scenes }))
    }

    async fn get_scene(
        &self,
        request: Request<String>,
    ) -> Result<Response<web::SceneResponse>, Status> {
        let id_str = request.into_inner();
        let id = Uuid::parse_str(&id_str)
            .map_err(|_| Status::invalid_argument(format!("invalid scene ID: {id_str}")))?;

        let config = self.config_handle.read().await;
        let scene = config
            .scenes
            .get(&scene::SceneId::from(id))
            .ok_or_else(|| Status::not_found(format!("scene not found: {id}")))?;

        Ok(Response::new(web::SceneResponse {
            scene: Some(scene_to_proto(scene)),
        }))
    }

    async fn add_fullscreen_scene(
        &self,
        request: Request<web::AddFullscreenSceneRequest>,
    ) -> Result<Response<String>, Status> {
        let _ = request;
        unimplemented!("filled in by Phase 3")
    }

    async fn add_combined_scene(&self, _request: Request<()>) -> Result<Response<String>, Status> {
        let scene = scene::Scene {
            id: scene::SceneId::generate(),
            enabled: true,
            cycle_duration: None,
            kind: scene::SceneKind::Combined,
            widgets: indexmap::IndexMap::new(),
        };
        let scene_id = scene.id.to_string();

        {
            let mut config = self.config_handle.write().await;
            config.scenes.insert(scene.id, scene);
            Self::save_config(&mut config).await?;
        }

        self.restore_active_scene().await;

        Ok(Response::new(scene_id))
    }

    async fn update_scene(
        &self,
        request: Request<web::UpdateSceneRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        let id =
            Uuid::parse_str(&req.id).map_err(|_| Status::invalid_argument("invalid scene ID"))?;

        if req.cycle_duration_sec == Some(0) {
            return Err(Status::invalid_argument(
                "cycle_duration_sec must be >= 1 when set",
            ));
        }

        let scene_id_key = scene::SceneId::from(id);
        let was_enabled;
        {
            let mut config = self.config_handle.write().await;
            let scene = config
                .scenes
                .get_mut(&scene_id_key)
                .ok_or_else(|| Status::not_found(format!("scene not found: {id}")))?;

            was_enabled = scene.enabled;
            scene.enabled = req.enabled;
            scene.cycle_duration = req
                .cycle_duration_sec
                .map(|s| std::time::Duration::from_secs(u64::from(s)));

            Self::save_config(&mut config).await?;
        }

        // Spawn/stop widgets based on enabled state change
        if was_enabled != req.enabled {
            let config = self.config_handle.read().await;
            if let Some(scene) = config.scenes.get(&scene_id_key) {
                if req.enabled {
                    self.coordinator.spawn_scene_widgets(scene).await;
                } else {
                    self.coordinator.stop_scene_widgets(scene).await;
                }
            }
        }

        self.restore_active_scene().await;

        Ok(Response::new(()))
    }

    async fn move_scene(
        &self,
        request: Request<web::MoveSceneRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        let id =
            Uuid::parse_str(&req.id).map_err(|_| Status::invalid_argument("invalid scene ID"))?;

        let mut config = self.config_handle.write().await;
        let scene_id = scene::SceneId::from(id);

        let current_idx = config
            .scenes
            .get_index_of(&scene_id)
            .ok_or_else(|| Status::not_found(format!("scene not found: {id}")))?;

        let target_idx = req.index as usize;
        if target_idx >= config.scenes.len() {
            return Err(Status::invalid_argument(format!(
                "target index {target_idx} out of bounds (scene count: {})",
                config.scenes.len()
            )));
        }
        config.scenes.move_index(current_idx, target_idx);

        Self::save_config(&mut config).await?;
        drop(config);

        self.restore_active_scene().await;

        Ok(Response::new(()))
    }

    async fn clone_scene(&self, request: Request<String>) -> Result<Response<String>, Status> {
        let id_str = request.into_inner();
        let id =
            Uuid::parse_str(&id_str).map_err(|_| Status::invalid_argument("invalid scene ID"))?;

        let mut config = self.config_handle.write().await;
        let source = config
            .scenes
            .get(&scene::SceneId::from(id))
            .ok_or_else(|| Status::not_found(format!("scene not found: {id}")))?
            .clone();

        let mut cloned = source;
        cloned.id = scene::SceneId::generate();
        // Give each widget a new ID
        let old_widgets = std::mem::take(&mut cloned.widgets);
        for (_, widget) in old_widgets {
            let new_widget = widget.clone_with_new_id();
            cloned.widgets.insert(new_widget.id, new_widget);
        }

        let cloned_id = cloned.id.to_string();
        let cloned_key = cloned.id;
        let cloned_enabled = cloned.enabled;

        // Insert right after the source scene
        let source_idx = config
            .scenes
            .get_index_of(&scene::SceneId::from(id))
            .expect("BUG: source scene just verified to exist");
        config.scenes.insert(cloned.id, cloned);
        let last_idx = config.scenes.len() - 1;
        config.scenes.move_index(last_idx, source_idx + 1);

        Self::save_config(&mut config).await?;
        drop(config);

        if cloned_enabled {
            let config = self.config_handle.read().await;
            if let Some(scene) = config.scenes.get(&cloned_key) {
                self.coordinator.spawn_scene_widgets(scene).await;
            }
        }
        self.restore_active_scene().await;

        Ok(Response::new(cloned_id))
    }

    async fn remove_scene(&self, request: Request<String>) -> Result<Response<()>, Status> {
        let id_str = request.into_inner();
        let id =
            Uuid::parse_str(&id_str).map_err(|_| Status::invalid_argument("invalid scene ID"))?;
        let scene_id_key = scene::SceneId::from(id);

        if self.preview_scene_id.lock().await.as_ref() == Some(&scene_id_key) {
            return Err(Status::failed_precondition(
                "scene is currently being previewed",
            ));
        }

        let removed_scene;
        {
            let mut config = self.config_handle.write().await;
            removed_scene = config
                .scenes
                .shift_remove(&scene_id_key)
                .ok_or_else(|| Status::not_found(format!("scene not found: {id}")))?;
            Self::save_config(&mut config).await?;
        }

        // Stop all widgets from the removed scene
        self.coordinator.stop_scene_widgets(&removed_scene).await;
        self.restore_active_scene().await;

        Ok(Response::new(()))
    }

    type PreviewSceneStream = BoxStream<'static, Result<(), Status>>;

    async fn preview_scene(
        &self,
        request: Request<String>,
    ) -> Result<Response<Self::PreviewSceneStream>, Status> {
        let id_str = request.into_inner();
        let id =
            Uuid::parse_str(&id_str).map_err(|_| Status::invalid_argument("invalid scene ID"))?;
        let scene_id = scene::SceneId::from(id);

        // Look the scene up, claim the preview slot, and kick off rendering
        // in a single config read so the scene can't vanish between the
        // existence check and the spawn.
        let scene_was_disabled = {
            let config = self.config_handle.read().await;
            let scene = config
                .scenes
                .get(&scene_id)
                .ok_or_else(|| Status::not_found(format!("scene not found: {scene_id}")))?;

            let mut preview = self.preview_scene_id.lock().await;
            if preview.is_some() {
                return Err(Status::resource_exhausted("scene preview already active"));
            }
            *preview = Some(scene_id);
            drop(preview);

            let disabled = !scene.enabled;
            if disabled {
                self.coordinator.spawn_scene_widgets(scene).await;
            }
            self.coordinator.set_active_scene(scene);
            disabled
        };

        // Guard that clears the preview slot and reverts the compositor
        // back to the first enabled scene when the client drops the stream.
        // If we spawned widgets to back a disabled preview, stop them too.
        struct PreviewGuard {
            coordinator: Arc<Coordinator>,
            config_handle: Arc<RwLock<ConfigHandle>>,
            preview_scene_id: Arc<Mutex<Option<scene::SceneId>>>,
            spawned_widgets: bool,
        }
        impl Drop for PreviewGuard {
            fn drop(&mut self) {
                let coordinator = Arc::clone(&self.coordinator);
                let config_handle = Arc::clone(&self.config_handle);
                let preview_scene_id = Arc::clone(&self.preview_scene_id);
                let spawned = self.spawned_widgets;
                tokio::spawn(async move {
                    let id = preview_scene_id.lock().await.take();
                    let config = config_handle.read().await;
                    if spawned
                        && let Some(id) = id.as_ref()
                        && let Some(scene) = config.scenes.get(id)
                    {
                        coordinator.stop_scene_widgets(scene).await;
                    }
                    coordinator.refresh_scene_cycling(&config.scenes);
                });
            }
        }

        let guard = PreviewGuard {
            coordinator: Arc::clone(&self.coordinator),
            config_handle: Arc::clone(&self.config_handle),
            preview_scene_id: Arc::clone(&self.preview_scene_id),
            spawned_widgets: scene_was_disabled,
        };

        // Heartbeat every 5s so the client sees a live stream; the guard
        // rides along in the unfold state and drops with the stream.
        let interval = time::interval(Duration::from_secs(5));
        let stream = futures::stream::unfold(
            (guard, IntervalStream::new(interval)),
            |(guard, mut ticks)| async move { ticks.next().await.map(|_| (Ok(()), (guard, ticks))) },
        );

        Ok(Response::new(stream.boxed()))
    }

    async fn remove_widget(
        &self,
        request: Request<web::RemoveWidgetRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        let scene_id = Uuid::parse_str(&req.scene_id)
            .map_err(|_| Status::invalid_argument("invalid scene ID"))?;
        let widget_id =
            Uuid::parse_str(&req.id).map_err(|_| Status::invalid_argument("invalid widget ID"))?;

        let instance_id = widget_id.to_string();
        {
            let mut config = self.config_handle.write().await;
            let scene = config
                .scenes
                .get_mut(&scene::SceneId::from(scene_id))
                .ok_or_else(|| Status::not_found(format!("scene not found: {scene_id}")))?;

            reject_remove_widget_in_fullscreen(&scene.kind)?;

            scene
                .widgets
                .shift_remove(&scene::WidgetId::from(widget_id))
                .ok_or_else(|| Status::not_found(format!("widget not found: {widget_id}")))?;

            Self::save_config(&mut config).await?;
        }

        self.coordinator.stop_widget(&instance_id).await;
        self.restore_active_scene().await;

        Ok(Response::new(()))
    }

    async fn get_scene_cycling(
        &self,
        _request: Request<()>,
    ) -> Result<Response<web::GetSceneCyclingResponse>, Status> {
        let config = self.config_handle.read().await;
        let cycling = config.scene_cycling();
        Ok(Response::new(web::GetSceneCyclingResponse {
            scene_cycling: Some(web::SceneCycling {
                automatic_cycling_enabled: cycling.automatic_cycling_enabled,
                automatic_cycling_default_duration_sec: u32::try_from(
                    cycling.automatic_cycling_default_duration.as_secs(),
                )
                .unwrap_or(u32::MAX),
                transition: match cycling.transition {
                    SceneCyclingTransition::Slide => web::SceneCyclingTransition::Slide.into(),
                    SceneCyclingTransition::Fade => web::SceneCyclingTransition::Fade.into(),
                },
            }),
        }))
    }

    async fn set_scene_cycling(
        &self,
        request: Request<web::SetSceneCyclingRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        let cycling = req
            .scene_cycling
            .ok_or_else(|| Status::invalid_argument("scene_cycling is required"))?;

        if cycling.automatic_cycling_default_duration_sec == 0 {
            return Err(Status::invalid_argument(
                "automatic_cycling_default_duration_sec must be >= 1",
            ));
        }

        let mut config = self.config_handle.write().await;
        config.set_scene_cycling(SceneCycling {
            automatic_cycling_enabled: cycling.automatic_cycling_enabled,
            automatic_cycling_default_duration: std::time::Duration::from_secs(u64::from(
                cycling.automatic_cycling_default_duration_sec,
            )),
            transition: match web::SceneCyclingTransition::try_from(cycling.transition) {
                Ok(web::SceneCyclingTransition::Fade) => SceneCyclingTransition::Fade,
                Ok(
                    web::SceneCyclingTransition::Slide | web::SceneCyclingTransition::Unspecified,
                )
                | Err(_) => SceneCyclingTransition::Slide,
            },
        });

        Self::save_config(&mut config).await?;

        Ok(Response::new(()))
    }

    async fn add_widget(
        &self,
        request: Request<web::AddWidgetRequest>,
    ) -> Result<Response<String>, Status> {
        let _ = request;
        unimplemented!("filled in by Phase 3")
    }

    async fn update_widget(
        &self,
        request: Request<web::UpdateWidgetRequest>,
    ) -> Result<Response<()>, Status> {
        let _ = request;
        unimplemented!("filled in by Phase 3")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_remove_widget_in_fullscreen_passes_for_combined() {
        assert!(reject_remove_widget_in_fullscreen(&scene::SceneKind::Combined).is_ok());
    }

    #[test]
    fn reject_remove_widget_in_fullscreen_rejects_with_failed_precondition() {
        let err = reject_remove_widget_in_fullscreen(&scene::SceneKind::Fullscreen)
            .expect_err("BUG: must reject fullscreen");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    }

    #[test]
    fn reject_update_widget_in_fullscreen_passes_for_combined() {
        let pos = web::WidgetPosition { row: 1, col: 2 };
        let result = reject_update_widget_in_fullscreen(
            &scene::SceneKind::Combined,
            pos,
            scene::WidgetSize::Small,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn reject_update_widget_in_fullscreen_allows_full_size_at_origin() {
        let pos = web::WidgetPosition { row: 0, col: 0 };
        let result = reject_update_widget_in_fullscreen(
            &scene::SceneKind::Fullscreen,
            pos,
            scene::WidgetSize::Full,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn reject_update_widget_in_fullscreen_rejects_position_change() {
        let pos = web::WidgetPosition { row: 1, col: 0 };
        let err = reject_update_widget_in_fullscreen(
            &scene::SceneKind::Fullscreen,
            pos,
            scene::WidgetSize::Full,
        )
        .expect_err("BUG: must reject moved fullscreen widget");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("move"));
    }

    #[test]
    fn reject_update_widget_in_fullscreen_rejects_size_change() {
        let pos = web::WidgetPosition { row: 0, col: 0 };
        let err = reject_update_widget_in_fullscreen(
            &scene::SceneKind::Fullscreen,
            pos,
            scene::WidgetSize::Small,
        )
        .expect_err("BUG: must reject resized fullscreen widget");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("resize"));
    }

    #[test]
    #[ignore = "re-enabled in Phase 3"]
    fn build_widget_params_keeps_default_when_key_omitted() {}

    #[test]
    #[ignore = "re-enabled in Phase 3"]
    fn build_widget_params_override_wins_over_default() {}

    #[test]
    #[ignore = "re-enabled in Phase 3"]
    fn build_widget_params_invalid_json_override_is_stored_as_string() {}

    #[test]
    #[ignore = "re-enabled in Phase 3"]
    fn param_type_timezone_maps_to_proto_timezone() {}
}
