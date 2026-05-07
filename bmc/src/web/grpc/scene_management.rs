// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::Arc;
use std::time::Duration;

use bmc_grpc::web;
use bmc_grpc::web::scene_management_service_server::SceneManagementService as GrpcSceneManagementService;
use bmc_ipc::SizeType;
use bmc_widget::{ParamDefinition, ParamKind};
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

pub(crate) fn build_widget_params(
    manifest: &bmc_widget::Manifest,
    overrides: &web::WidgetDataStruct,
) -> web::WidgetDataStruct {
    let mut fields = std::collections::HashMap::new();
    for (key, def) in &manifest.params {
        let value = overrides
            .fields
            .get(key.as_str())
            .cloned()
            .unwrap_or_else(|| param_default_to_widget_data_value(&def.kind));
        fields.insert(key.as_str().to_owned(), value);
    }
    web::WidgetDataStruct { fields }
}

fn param_default_to_widget_data_value(kind: &ParamKind) -> web::WidgetDataValue {
    use web::widget_data_value::Kind as VK;
    let arm = match kind {
        ParamKind::String {
            default_value: Some(d),
            ..
        } => VK::StringValue(d.clone()),
        ParamKind::Double {
            default_value: Some(d),
            ..
        } => VK::DoubleValue(*d),
        ParamKind::Integer {
            default_value: Some(d),
            ..
        } => VK::IntegerValue(*d),
        ParamKind::Boolean {
            default_value: Some(b),
        } => VK::BooleanValue(*b),
        ParamKind::Timezone {
            default_value: Some(s),
        } => VK::StringValue(s.clone()),
        ParamKind::String { .. }
        | ParamKind::Double { .. }
        | ParamKind::Integer { .. }
        | ParamKind::Boolean { .. }
        | ParamKind::Timezone { .. } => VK::NullValue(web::widget_data_value::Null {}),
    };
    web::WidgetDataValue { kind: Some(arm) }
}

fn widget_data_struct_to_json(s: &web::WidgetDataStruct) -> serde_json::Value {
    let map = s
        .fields
        .iter()
        .map(|(k, v)| (k.clone(), widget_data_value_to_json(v)))
        .collect();
    serde_json::Value::Object(map)
}

fn widget_data_value_to_json(v: &web::WidgetDataValue) -> serde_json::Value {
    use web::widget_data_value::Kind;
    match &v.kind {
        None | Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::BooleanValue(b)) => serde_json::Value::Bool(*b),
        Some(Kind::IntegerValue(i)) => serde_json::Value::Number((*i).into()),
        Some(Kind::DoubleValue(d)) => {
            let n = serde_json::Number::from_f64(*d)
                .expect("BUG: validate_widget_params must reject non-finite double_value");
            serde_json::Value::Number(n)
        }
        Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
    }
}

fn json_to_widget_data_struct(
    json: &serde_json::Value,
    manifest_kinds: &std::collections::HashMap<String, ParamKind>,
) -> web::WidgetDataStruct {
    let mut fields = std::collections::HashMap::new();
    if let Some(obj) = json.as_object() {
        for (k, v) in obj {
            if let Some(value) = json_to_widget_data_value(v, k, manifest_kinds.get(k)) {
                fields.insert(k.clone(), value);
            }
        }
    }
    web::WidgetDataStruct { fields }
}

fn json_to_widget_data_value(
    v: &serde_json::Value,
    key: &str,
    kind_hint: Option<&ParamKind>,
) -> Option<web::WidgetDataValue> {
    use web::widget_data_value::Kind;
    let arm = match (v, kind_hint) {
        (serde_json::Value::Null, _) => Kind::NullValue(web::widget_data_value::Null {}),
        (serde_json::Value::Bool(b), _) => Kind::BooleanValue(*b),
        (serde_json::Value::String(s), _) => Kind::StringValue(s.clone()),
        (serde_json::Value::Number(n), Some(ParamKind::Integer { .. })) => {
            let Some(i64v) = n.as_i64() else {
                tracing::warn!(key, value = %n, "stored Integer param is not representable as i64; omitting");
                return None;
            };
            let Ok(i32v) = i32::try_from(i64v) else {
                tracing::warn!(key, value = %n, "stored Integer param overflows i32; omitting");
                return None;
            };
            Kind::IntegerValue(i32v)
        }
        (serde_json::Value::Number(n), Some(ParamKind::Double { .. })) => {
            let Some(d) = n.as_f64().filter(|d| d.is_finite()) else {
                tracing::warn!(key, value = %n, "stored Double param is not finite; omitting");
                return None;
            };
            Kind::DoubleValue(d)
        }
        (serde_json::Value::Number(n), _) if n.is_i64() => {
            let Some(i) = n.as_i64().and_then(|i| i32::try_from(i).ok()) else {
                tracing::warn!(key, value = %n, "stored integer-shaped value overflows i32 (no manifest); omitting");
                return None;
            };
            Kind::IntegerValue(i)
        }
        (serde_json::Value::Number(n), _) => {
            let Some(d) = n.as_f64().filter(|d| d.is_finite()) else {
                tracing::warn!(key, value = %n, "stored numeric value is not finite (no manifest); omitting");
                return None;
            };
            Kind::DoubleValue(d)
        }
        (serde_json::Value::Array(_) | serde_json::Value::Object(_), _) => {
            tracing::warn!(
                key,
                "stored param is array/object — manifest schema disallows; omitting"
            );
            return None;
        }
    };
    Some(web::WidgetDataValue { kind: Some(arm) })
}

fn size_type_to_proto(size: SizeType) -> i32 {
    match size {
        SizeType::Small => web::WidgetSize::Small.into(),
        SizeType::Medium => web::WidgetSize::Medium.into(),
        SizeType::Large => web::WidgetSize::Large.into(),
        SizeType::Full => web::WidgetSize::Full.into(),
    }
}

fn param_definition_to_proto(key: &str, def: &ParamDefinition) -> web::ManifestParamDefinition {
    use web::manifest_param_definition::Kind as PK;
    let kind = match &def.kind {
        ParamKind::String {
            format,
            enum_values,
            default_value,
        } => PK::ParamString(web::ParamString {
            format: format.map(string_format_to_proto).map(i32::from),
            enum_values: enum_values
                .iter()
                .map(|o| web::StringOption {
                    value: o.value.clone(),
                    label: o.label.clone(),
                })
                .collect(),
            default_value: default_value.clone(),
        }),
        ParamKind::Double {
            min,
            max,
            step,
            enum_values,
            default_value,
        } => PK::ParamDouble(web::ParamDouble {
            min: *min,
            max: *max,
            step: *step,
            enum_values: enum_values
                .iter()
                .map(|o| web::DoubleOption {
                    value: o.value,
                    label: o.label.clone(),
                })
                .collect(),
            default_value: *default_value,
        }),
        ParamKind::Integer {
            min,
            max,
            step,
            enum_values,
            default_value,
        } => PK::ParamInteger(web::ParamInteger {
            min: *min,
            max: *max,
            step: *step,
            enum_values: enum_values
                .iter()
                .map(|o| web::IntegerOption {
                    value: o.value,
                    label: o.label.clone(),
                })
                .collect(),
            default_value: *default_value,
        }),
        ParamKind::Boolean { default_value } => PK::ParamBoolean(web::ParamBoolean {
            default_value: *default_value,
        }),
        ParamKind::Timezone { default_value } => PK::ParamTimezone(web::ParamTimezone {
            default_value: default_value.clone(),
        }),
    };
    web::ManifestParamDefinition {
        key: key.to_owned(),
        name: def.name.clone(),
        description: def.description.clone(),
        is_optional: def.is_optional,
        kind: Some(kind),
    }
}

fn string_format_to_proto(f: bmc_widget::StringFormat) -> web::StringFormat {
    use bmc_widget::StringFormat as F;
    match f {
        F::Date => web::StringFormat::Date,
        F::Time => web::StringFormat::Time,
        F::Email => web::StringFormat::Email,
        F::Uri => web::StringFormat::Uri,
    }
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
            .map(|(key, param)| param_definition_to_proto(key.as_str(), param))
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

pub(crate) enum ValidateMode {
    Add,
    Update,
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "enum is cheap; by-value keeps call-site ergonomic"
)]
pub(crate) fn validate_widget_params(
    manifest: &bmc_widget::Manifest,
    params: &web::WidgetDataStruct,
    mode: ValidateMode,
) -> Result<(), Status> {
    use web::widget_data_value::Kind as VK;

    if matches!(mode, ValidateMode::Update) {
        for key in manifest.params.keys() {
            if !params.fields.contains_key(key.as_str()) {
                return Err(Status::invalid_argument(format!(
                    "param {:?}: missing in UpdateWidget request",
                    key.as_str()
                )));
            }
        }
    }

    for (key, value) in &params.fields {
        let def = manifest
            .params
            .get(key.as_str())
            .ok_or_else(|| Status::invalid_argument(format!("param {key:?}: unknown")))?;

        let kind = value.kind.as_ref().ok_or_else(|| {
            Status::invalid_argument(format!("param {key:?}: WidgetDataValue.kind unset"))
        })?;

        if matches!(kind, VK::NullValue(_)) {
            if !def.is_optional {
                return Err(Status::invalid_argument(format!(
                    "param {key:?}: null_value on required param"
                )));
            }
            continue;
        }

        validate_widget_param_value(key, &def.kind, kind)?;
    }
    Ok(())
}

fn validate_widget_param_value(
    key: &str,
    param_kind: &ParamKind,
    kind: &web::widget_data_value::Kind,
) -> Result<(), Status> {
    use web::widget_data_value::Kind as VK;
    match (param_kind, kind) {
        (ParamKind::String { enum_values, .. }, VK::StringValue(s)) => {
            if !enum_values.is_empty() && !enum_values.iter().any(|o| &o.value == s) {
                return Err(Status::invalid_argument(format!(
                    "param {key:?}: value not in enum_values"
                )));
            }
        }
        (ParamKind::Timezone { .. }, VK::StringValue(_))
        | (ParamKind::Boolean { .. }, VK::BooleanValue(_)) => {}
        (
            ParamKind::Integer {
                min,
                max,
                enum_values,
                ..
            },
            VK::IntegerValue(i),
        ) => {
            if let Some(lo) = min
                && i < lo
            {
                return Err(Status::invalid_argument(format!(
                    "param {key:?}: {i} < min {lo}"
                )));
            }
            if let Some(hi) = max
                && i > hi
            {
                return Err(Status::invalid_argument(format!(
                    "param {key:?}: {i} > max {hi}"
                )));
            }
            if !enum_values.is_empty() && !enum_values.iter().any(|o| o.value == *i) {
                return Err(Status::invalid_argument(format!(
                    "param {key:?}: value not in enum_values"
                )));
            }
        }
        (
            ParamKind::Double {
                min,
                max,
                enum_values,
                ..
            },
            VK::DoubleValue(d),
        ) => {
            if !d.is_finite() {
                return Err(Status::invalid_argument(format!(
                    "param {key:?}: double_value must be finite (got {d})"
                )));
            }
            if let Some(lo) = min
                && d < lo
            {
                return Err(Status::invalid_argument(format!(
                    "param {key:?}: {d} < min {lo}"
                )));
            }
            if let Some(hi) = max
                && d > hi
            {
                return Err(Status::invalid_argument(format!(
                    "param {key:?}: {d} > max {hi}"
                )));
            }
            if !enum_values.is_empty()
                && !enum_values.iter().any(|o| o.value.to_bits() == d.to_bits())
            {
                return Err(Status::invalid_argument(format!(
                    "param {key:?}: value not in enum_values"
                )));
            }
        }
        _ => {
            return Err(Status::invalid_argument(format!(
                "param {key:?}: WidgetDataValue arm does not match declared variant"
            )));
        }
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
        let req = request.into_inner();
        let config = req
            .config
            .ok_or_else(|| Status::invalid_argument("config is required"))?;

        let widget_uid = Uuid::parse_str(&config.widget_uid)
            .map_err(|_| Status::invalid_argument("invalid widget UID"))?;

        let info = self
            .widget_registry
            .get(&widget_uid)
            .ok_or_else(|| Status::failed_precondition("widget manifest not installed"))?;
        let manifest = &info.manifest;

        let params = config.params.unwrap_or_default();
        validate_widget_params(manifest, &params, ValidateMode::Add)?;
        let resolved = build_widget_params(manifest, &params);
        let params_json = widget_data_struct_to_json(&resolved);

        let scene = scene::Scene::fullscreen(widget_uid, params_json);
        let scene_id = scene.id.to_string();

        let scene_enabled = scene.enabled;
        let scene_key = scene.id;

        {
            let mut config = self.config_handle.write().await;
            config.scenes.insert(scene.id, scene);
            Self::save_config(&mut config).await?;
        }

        if scene_enabled {
            let config = self.config_handle.read().await;
            if let Some(scene) = config.scenes.get(&scene_key) {
                self.coordinator.spawn_scene_widgets(scene).await;
            }
        }
        self.restore_active_scene().await;

        Ok(Response::new(scene_id))
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
        let req = request.into_inner();
        let scene_id = Uuid::parse_str(&req.scene_id)
            .map_err(|_| Status::invalid_argument("invalid scene ID"))?;

        let position = proto_position_to_scene(req.position.unwrap_or_default())?;
        let size = proto_size_to_scene(req.size)?;

        let config_req = req
            .config
            .ok_or_else(|| Status::invalid_argument("config is required"))?;

        let widget_uid = Uuid::parse_str(&config_req.widget_uid)
            .map_err(|_| Status::invalid_argument("invalid widget UID"))?;

        let info = self
            .widget_registry
            .get(&widget_uid)
            .ok_or_else(|| Status::failed_precondition("widget manifest not installed"))?;
        let manifest = &info.manifest;

        let params = config_req.params.unwrap_or_default();
        validate_widget_params(manifest, &params, ValidateMode::Add)?;
        let resolved = build_widget_params(manifest, &params);
        let params_json = widget_data_struct_to_json(&resolved);

        let widget = scene::Widget::new(widget_uid, params_json, position, size);
        let widget_id = widget.id.to_string();

        let scene_id_key = scene::SceneId::from(scene_id);
        let showing = self.scene_is_showing(&scene_id_key).await;

        {
            let mut config = self.config_handle.write().await;
            let scene = config
                .scenes
                .get_mut(&scene_id_key)
                .ok_or_else(|| Status::not_found(format!("scene not found: {scene_id}")))?;

            validate_widget_placement(scene, &widget, None)?;

            scene.widgets.insert(widget.id, widget.clone());
            Self::save_config(&mut config).await?;
        }

        if showing {
            self.try_spawn_widget(&scene_id_key, &widget).await;
        }
        self.restore_active_scene().await;

        Ok(Response::new(widget_id))
    }

    async fn update_widget(
        &self,
        request: Request<web::UpdateWidgetRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        let scene_id = Uuid::parse_str(&req.scene_id)
            .map_err(|_| Status::invalid_argument("invalid scene ID"))?;
        let widget_id =
            Uuid::parse_str(&req.id).map_err(|_| Status::invalid_argument("invalid widget ID"))?;

        let proto_position = req.position.unwrap_or_default();
        let position = proto_position_to_scene(proto_position)?;
        let size = proto_size_to_scene(req.size)?;

        let scene_id_key = scene::SceneId::from(scene_id);
        let widget_id_key = scene::WidgetId::from(widget_id);

        {
            let mut config = self.config_handle.write().await;
            let scene = config
                .scenes
                .get_mut(&scene_id_key)
                .ok_or_else(|| Status::not_found(format!("scene not found: {scene_id}")))?;

            reject_update_widget_in_fullscreen(&scene.kind, proto_position, size)?;

            let widget_uid = scene
                .widgets
                .get(&widget_id_key)
                .ok_or_else(|| Status::not_found(format!("widget not found: {widget_id}")))?
                .widget_type_id;

            let info = self
                .widget_registry
                .get(&widget_uid)
                .ok_or_else(|| Status::failed_precondition("widget manifest not installed"))?;
            let manifest = &info.manifest;

            let params = req.params.unwrap_or_default();
            validate_widget_params(manifest, &params, ValidateMode::Update)?;
            let params_json = widget_data_struct_to_json(&params);

            let widget = scene
                .widgets
                .get_mut(&widget_id_key)
                .expect("BUG: widget was just found above");
            widget.position = position;
            widget.size = size;
            widget.params = params_json;

            let widget_snapshot = widget.clone();
            validate_widget_placement(scene, &widget_snapshot, Some(widget_id_key))?;

            Self::save_config(&mut config).await?;
        }

        self.restore_active_scene().await;

        Ok(Response::new(()))
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
    fn build_widget_params_seeds_required_with_default() {
        use web::widget_data_value::Kind as VK;
        let manifest = single_param_manifest(
            "name",
            ParamKind::String {
                format: None,
                enum_values: vec![],
                default_value: Some("hello".into()),
            },
            false,
        );
        let resolved = build_widget_params(&manifest, &web::WidgetDataStruct::default());
        assert_eq!(resolved.fields.len(), 1);
        let v = &resolved.fields["name"];
        let Some(VK::StringValue(s)) = &v.kind else {
            panic!("BUG: expected StringValue")
        };
        assert_eq!(s, "hello");
    }

    #[test]
    fn build_widget_params_seeds_optional_no_default_with_null() {
        use web::widget_data_value::Kind as VK;
        let manifest = single_param_manifest(
            "name",
            ParamKind::String {
                format: None,
                enum_values: vec![],
                default_value: None,
            },
            true,
        );
        let resolved = build_widget_params(&manifest, &web::WidgetDataStruct::default());
        let v = &resolved.fields["name"];
        assert!(matches!(v.kind, Some(VK::NullValue(_))));
    }

    #[test]
    fn build_widget_params_override_wins() {
        use web::widget_data_value::Kind as VK;
        let manifest = single_param_manifest(
            "name",
            ParamKind::String {
                format: None,
                enum_values: vec![],
                default_value: Some("hello".into()),
            },
            false,
        );
        let mut overrides = web::WidgetDataStruct::default();
        overrides
            .fields
            .insert("name".to_owned(), wdv_string("world"));
        let resolved = build_widget_params(&manifest, &overrides);
        let v = &resolved.fields["name"];
        let Some(VK::StringValue(s)) = &v.kind else {
            panic!("BUG: expected StringValue")
        };
        assert_eq!(s, "world");
    }

    #[test]
    fn build_widget_params_seeds_each_kind_with_default() {
        use web::widget_data_value::Kind as VK;
        let manifest = manifest_with_params(&[
            (
                "s",
                ParamKind::String {
                    format: None,
                    enum_values: vec![],
                    default_value: Some("x".into()),
                },
                false,
            ),
            (
                "i",
                ParamKind::Integer {
                    min: None,
                    max: None,
                    step: None,
                    enum_values: vec![],
                    default_value: Some(7),
                },
                false,
            ),
            (
                "d",
                ParamKind::Double {
                    min: None,
                    max: None,
                    step: None,
                    enum_values: vec![],
                    default_value: Some(2.5),
                },
                false,
            ),
            (
                "b",
                ParamKind::Boolean {
                    default_value: Some(true),
                },
                false,
            ),
            (
                "t",
                ParamKind::Timezone {
                    default_value: Some("UTC".into()),
                },
                false,
            ),
        ]);
        let resolved = build_widget_params(&manifest, &web::WidgetDataStruct::default());
        assert_eq!(resolved.fields.len(), 5);
        assert!(matches!(
            resolved.fields["s"].kind,
            Some(VK::StringValue(_))
        ));
        assert!(matches!(
            resolved.fields["i"].kind,
            Some(VK::IntegerValue(7))
        ));
        assert!(matches!(
            resolved.fields["d"].kind,
            Some(VK::DoubleValue(_))
        ));
        assert!(matches!(
            resolved.fields["b"].kind,
            Some(VK::BooleanValue(true))
        ));
        assert!(matches!(
            resolved.fields["t"].kind,
            Some(VK::StringValue(_))
        ));
    }

    #[test]
    fn widget_data_struct_to_json_round_trips_each_arm() {
        use bmc_widget::ParamKind;
        let manifest_kinds = std::collections::HashMap::from([
            (
                "s".to_owned(),
                ParamKind::String {
                    format: None,
                    enum_values: vec![],
                    default_value: Some("x".into()),
                },
            ),
            (
                "i".to_owned(),
                ParamKind::Integer {
                    min: None,
                    max: None,
                    step: None,
                    enum_values: vec![],
                    default_value: Some(2),
                },
            ),
            (
                "d".to_owned(),
                ParamKind::Double {
                    min: None,
                    max: None,
                    step: None,
                    enum_values: vec![],
                    default_value: Some(2.5),
                },
            ),
            (
                "b".to_owned(),
                ParamKind::Boolean {
                    default_value: Some(true),
                },
            ),
        ]);

        let s = web::WidgetDataStruct {
            fields: [
                ("s".to_owned(), wdv_string("hello")),
                ("i".to_owned(), wdv_integer(42)),
                ("d".to_owned(), wdv_double(2.5)),
                ("b".to_owned(), wdv_boolean(false)),
            ]
            .into_iter()
            .collect(),
        };

        let json = widget_data_struct_to_json(&s);
        let back = json_to_widget_data_struct(&json, &manifest_kinds);
        assert_eq!(back.fields, s.fields);
    }

    #[test]
    fn widget_data_value_to_json_null_arm_is_json_null() {
        let v = wdv_null();
        let j = widget_data_value_to_json(&v);
        assert!(j.is_null());
    }

    #[test]
    fn json_to_widget_data_struct_falls_back_to_shape_inference_without_manifest() {
        use web::widget_data_value::Kind;
        let manifest_kinds: std::collections::HashMap<String, bmc_widget::ParamKind> =
            std::collections::HashMap::new();
        let json = serde_json::json!({
            "s": "hello",
            "i": 5,
            "d": 1.5,
            "b": true,
            "n": serde_json::Value::Null,
        });
        let back = json_to_widget_data_struct(&json, &manifest_kinds);
        assert!(matches!(back.fields["s"].kind, Some(Kind::StringValue(_))));
        assert!(matches!(back.fields["i"].kind, Some(Kind::IntegerValue(5))));
        assert!(matches!(back.fields["d"].kind, Some(Kind::DoubleValue(_))));
        assert!(matches!(
            back.fields["b"].kind,
            Some(Kind::BooleanValue(true))
        ));
        assert!(matches!(back.fields["n"].kind, Some(Kind::NullValue(_))));
    }

    fn wdv_string(s: &str) -> web::WidgetDataValue {
        web::WidgetDataValue {
            kind: Some(web::widget_data_value::Kind::StringValue(s.to_owned())),
        }
    }
    fn wdv_integer(i: i32) -> web::WidgetDataValue {
        web::WidgetDataValue {
            kind: Some(web::widget_data_value::Kind::IntegerValue(i)),
        }
    }
    fn wdv_double(d: f64) -> web::WidgetDataValue {
        web::WidgetDataValue {
            kind: Some(web::widget_data_value::Kind::DoubleValue(d)),
        }
    }
    fn wdv_boolean(b: bool) -> web::WidgetDataValue {
        web::WidgetDataValue {
            kind: Some(web::widget_data_value::Kind::BooleanValue(b)),
        }
    }
    fn wdv_null() -> web::WidgetDataValue {
        web::WidgetDataValue {
            kind: Some(web::widget_data_value::Kind::NullValue(
                web::widget_data_value::Null {},
            )),
        }
    }
    fn wdv_unset_kind() -> web::WidgetDataValue {
        web::WidgetDataValue { kind: None }
    }

    fn single_param_manifest(
        key: &str,
        kind: bmc_widget::ParamKind,
        is_optional: bool,
    ) -> bmc_widget::Manifest {
        let param = bmc_widget::ParamDefinition {
            name: "Test".into(),
            description: None,
            is_optional,
            kind,
        };
        let pk: bmc_widget::ParamKey =
            serde_json::from_str(&format!("\"{key}\"")).expect("BUG: valid key");
        let mut params = std::collections::HashMap::new();
        params.insert(pk, param);
        bmc_widget::Manifest {
            uid: uuid::Uuid::new_v4(),
            version: semver::Version::new(1, 0, 0),
            name: "T".into(),
            description: "T".into(),
            author: None,
            binary: std::path::PathBuf::from("bin/test"),
            settings: vec![],
            sizes: vec![bmc_ipc::SizeType::Small],
            params,
        }
    }

    fn manifest_with_params(
        entries: &[(&str, bmc_widget::ParamKind, bool)],
    ) -> bmc_widget::Manifest {
        let mut params = std::collections::HashMap::new();
        for (key, kind, is_optional) in entries {
            let pk: bmc_widget::ParamKey =
                serde_json::from_str(&format!("\"{key}\"")).expect("BUG: valid key");
            let param = bmc_widget::ParamDefinition {
                name: "Test".into(),
                description: None,
                is_optional: *is_optional,
                kind: kind.clone(),
            };
            params.insert(pk, param);
        }
        bmc_widget::Manifest {
            uid: uuid::Uuid::new_v4(),
            version: semver::Version::new(1, 0, 0),
            name: "T".into(),
            description: "T".into(),
            author: None,
            binary: std::path::PathBuf::from("bin/test"),
            settings: vec![],
            sizes: vec![bmc_ipc::SizeType::Small],
            params,
        }
    }

    fn fields_one(key: &str, value: web::WidgetDataValue) -> web::WidgetDataStruct {
        web::WidgetDataStruct {
            fields: [(key.to_owned(), value)].into_iter().collect(),
        }
    }

    #[test]
    fn validate_widget_params_string_string_value_accepts() {
        let manifest = single_param_manifest(
            "color",
            bmc_widget::ParamKind::String {
                format: None,
                enum_values: vec![],
                default_value: Some("red".into()),
            },
            false,
        );
        let params = fields_one("color", wdv_string("blue"));
        assert!(validate_widget_params(&manifest, &params, ValidateMode::Add).is_ok());
    }

    #[test]
    fn validate_widget_params_string_double_value_rejects() {
        let manifest = single_param_manifest(
            "color",
            bmc_widget::ParamKind::String {
                format: None,
                enum_values: vec![],
                default_value: Some("red".into()),
            },
            false,
        );
        let params = fields_one("color", wdv_double(1.0));
        let err = validate_widget_params(&manifest, &params, ValidateMode::Add)
            .expect_err("BUG: must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn validate_widget_params_double_for_integer_rejects() {
        let manifest = single_param_manifest(
            "count",
            bmc_widget::ParamKind::Integer {
                min: None,
                max: None,
                step: None,
                enum_values: vec![],
                default_value: Some(0),
            },
            false,
        );
        let params = fields_one("count", wdv_double(5.0));
        let err = validate_widget_params(&manifest, &params, ValidateMode::Add)
            .expect_err("BUG: must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn validate_widget_params_integer_for_double_rejects() {
        let manifest = single_param_manifest(
            "ratio",
            bmc_widget::ParamKind::Double {
                min: None,
                max: None,
                step: None,
                enum_values: vec![],
                default_value: Some(0.5),
            },
            false,
        );
        let params = fields_one("ratio", wdv_integer(1));
        let err = validate_widget_params(&manifest, &params, ValidateMode::Add)
            .expect_err("BUG: must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn validate_widget_params_unset_kind_rejects() {
        let manifest = single_param_manifest(
            "flag",
            bmc_widget::ParamKind::Boolean {
                default_value: Some(false),
            },
            false,
        );
        let params = fields_one("flag", wdv_unset_kind());
        let err = validate_widget_params(&manifest, &params, ValidateMode::Add)
            .expect_err("BUG: must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn validate_widget_params_null_value_on_required_rejects() {
        let manifest = single_param_manifest(
            "name",
            bmc_widget::ParamKind::String {
                format: None,
                enum_values: vec![],
                default_value: Some("x".into()),
            },
            false,
        );
        let params = fields_one("name", wdv_null());
        let err = validate_widget_params(&manifest, &params, ValidateMode::Add)
            .expect_err("BUG: must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn validate_widget_params_null_value_on_optional_accepts() {
        let manifest = single_param_manifest(
            "label",
            bmc_widget::ParamKind::String {
                format: None,
                enum_values: vec![],
                default_value: None,
            },
            true,
        );
        let params = fields_one("label", wdv_null());
        assert!(validate_widget_params(&manifest, &params, ValidateMode::Add).is_ok());
    }

    #[test]
    fn validate_widget_params_double_nan_rejects() {
        let manifest = single_param_manifest(
            "val",
            bmc_widget::ParamKind::Double {
                min: None,
                max: None,
                step: None,
                enum_values: vec![],
                default_value: Some(1.0),
            },
            false,
        );
        let params = fields_one("val", wdv_double(f64::NAN));
        let err = validate_widget_params(&manifest, &params, ValidateMode::Add)
            .expect_err("BUG: must reject NaN");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn validate_widget_params_double_inf_rejects() {
        let manifest = single_param_manifest(
            "val",
            bmc_widget::ParamKind::Double {
                min: None,
                max: None,
                step: None,
                enum_values: vec![],
                default_value: Some(1.0),
            },
            false,
        );
        let params = fields_one("val", wdv_double(f64::INFINITY));
        let err = validate_widget_params(&manifest, &params, ValidateMode::Add)
            .expect_err("BUG: must reject Inf");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn validate_widget_params_integer_below_min_rejects() {
        let manifest = single_param_manifest(
            "n",
            bmc_widget::ParamKind::Integer {
                min: Some(5),
                max: None,
                step: None,
                enum_values: vec![],
                default_value: Some(5),
            },
            false,
        );
        let params = fields_one("n", wdv_integer(4));
        let err = validate_widget_params(&manifest, &params, ValidateMode::Add)
            .expect_err("BUG: must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn validate_widget_params_integer_above_max_rejects() {
        let manifest = single_param_manifest(
            "n",
            bmc_widget::ParamKind::Integer {
                min: None,
                max: Some(10),
                step: None,
                enum_values: vec![],
                default_value: Some(5),
            },
            false,
        );
        let params = fields_one("n", wdv_integer(11));
        let err = validate_widget_params(&manifest, &params, ValidateMode::Add)
            .expect_err("BUG: must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn validate_widget_params_double_below_min_rejects() {
        let manifest = single_param_manifest(
            "ratio",
            bmc_widget::ParamKind::Double {
                min: Some(0.0),
                max: Some(1.0),
                step: None,
                enum_values: vec![],
                default_value: Some(0.5),
            },
            false,
        );
        let params = fields_one("ratio", wdv_double(-0.1));
        let err = validate_widget_params(&manifest, &params, ValidateMode::Add)
            .expect_err("BUG: must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn validate_widget_params_enum_value_not_in_options_rejects() {
        let manifest = single_param_manifest(
            "style",
            bmc_widget::ParamKind::String {
                format: None,
                enum_values: vec![
                    bmc_widget::StringOption {
                        value: "dark".into(),
                        label: "Dark".into(),
                    },
                    bmc_widget::StringOption {
                        value: "light".into(),
                        label: "Light".into(),
                    },
                ],
                default_value: Some("dark".into()),
            },
            false,
        );
        let params = fields_one("style", wdv_string("solarized"));
        let err = validate_widget_params(&manifest, &params, ValidateMode::Add)
            .expect_err("BUG: must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn validate_widget_params_enum_value_in_options_accepts() {
        let manifest = single_param_manifest(
            "style",
            bmc_widget::ParamKind::String {
                format: None,
                enum_values: vec![
                    bmc_widget::StringOption {
                        value: "dark".into(),
                        label: "Dark".into(),
                    },
                    bmc_widget::StringOption {
                        value: "light".into(),
                        label: "Light".into(),
                    },
                ],
                default_value: Some("dark".into()),
            },
            false,
        );
        let params = fields_one("style", wdv_string("light"));
        assert!(validate_widget_params(&manifest, &params, ValidateMode::Add).is_ok());
    }

    #[test]
    fn validate_widget_params_unknown_key_rejects() {
        let manifest = single_param_manifest(
            "known",
            bmc_widget::ParamKind::Boolean {
                default_value: Some(true),
            },
            false,
        );
        let params = fields_one("unknown", wdv_boolean(true));
        let err = validate_widget_params(&manifest, &params, ValidateMode::Add)
            .expect_err("BUG: must reject unknown key");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn validate_widget_params_update_missing_key_rejects() {
        let manifest = single_param_manifest(
            "flag",
            bmc_widget::ParamKind::Boolean {
                default_value: Some(false),
            },
            false,
        );
        let params = web::WidgetDataStruct {
            fields: std::collections::HashMap::new(),
        };
        let err = validate_widget_params(&manifest, &params, ValidateMode::Update)
            .expect_err("BUG: Update must require all keys");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn validate_widget_params_add_missing_key_accepts() {
        let manifest = single_param_manifest(
            "flag",
            bmc_widget::ParamKind::Boolean {
                default_value: Some(false),
            },
            false,
        );
        let params = web::WidgetDataStruct {
            fields: std::collections::HashMap::new(),
        };
        assert!(validate_widget_params(&manifest, &params, ValidateMode::Add).is_ok());
    }

    #[test]
    fn param_definition_to_proto_string_with_enum() {
        use bmc_widget::{ParamDefinition, ParamKind, StringOption};
        use web::manifest_param_definition::Kind;
        let p = ParamDefinition {
            name: "Style".into(),
            description: None,
            is_optional: false,
            kind: ParamKind::String {
                format: None,
                enum_values: vec![
                    StringOption {
                        value: "a".into(),
                        label: "A".into(),
                    },
                    StringOption {
                        value: "b".into(),
                        label: "B".into(),
                    },
                ],
                default_value: Some("a".into()),
            },
        };
        let proto = param_definition_to_proto("style", &p);
        assert_eq!(proto.key, "style");
        assert!(!proto.is_optional);
        let Some(Kind::ParamString(ps)) = proto.kind else {
            panic!("BUG: expected param_string arm");
        };
        assert_eq!(ps.default_value.as_deref(), Some("a"));
        assert_eq!(ps.enum_values.len(), 2);
        assert_eq!(ps.enum_values[0].value, "a");
    }

    #[test]
    fn param_definition_to_proto_double() {
        use bmc_widget::{ParamDefinition, ParamKind};
        use web::manifest_param_definition::Kind;
        let p = ParamDefinition {
            name: "Brightness".into(),
            description: Some("Brightness level".into()),
            is_optional: false,
            kind: ParamKind::Double {
                min: Some(0.0),
                max: Some(1.0),
                step: Some(0.1),
                enum_values: vec![],
                default_value: Some(0.5),
            },
        };
        let proto = param_definition_to_proto("brightness", &p);
        assert_eq!(proto.key, "brightness");
        let Some(Kind::ParamDouble(pd)) = proto.kind else {
            panic!("BUG: expected param_double arm");
        };
        assert_eq!(pd.default_value, Some(0.5));
        assert_eq!(pd.min, Some(0.0));
        assert_eq!(pd.max, Some(1.0));
        assert_eq!(pd.step, Some(0.1));
    }

    #[test]
    fn param_definition_to_proto_integer() {
        use bmc_widget::{ParamDefinition, ParamKind};
        use web::manifest_param_definition::Kind;
        let p = ParamDefinition {
            name: "Count".into(),
            description: None,
            is_optional: false,
            kind: ParamKind::Integer {
                min: Some(0),
                max: Some(10),
                step: Some(1),
                enum_values: vec![],
                default_value: Some(5),
            },
        };
        let proto = param_definition_to_proto("count", &p);
        assert_eq!(proto.key, "count");
        let Some(Kind::ParamInteger(pi)) = proto.kind else {
            panic!("BUG: expected param_integer arm");
        };
        assert_eq!(pi.default_value, Some(5));
        assert_eq!(pi.min, Some(0));
        assert_eq!(pi.max, Some(10));
        assert_eq!(pi.step, Some(1));
    }

    #[test]
    fn param_definition_to_proto_boolean() {
        use bmc_widget::{ParamDefinition, ParamKind};
        use web::manifest_param_definition::Kind;
        let p = ParamDefinition {
            name: "Show seconds".into(),
            description: None,
            is_optional: false,
            kind: ParamKind::Boolean {
                default_value: Some(true),
            },
        };
        let proto = param_definition_to_proto("show-seconds", &p);
        assert_eq!(proto.key, "show-seconds");
        let Some(Kind::ParamBoolean(pb)) = proto.kind else {
            panic!("BUG: expected param_boolean arm");
        };
        assert_eq!(pb.default_value, Some(true));
    }

    #[test]
    fn param_definition_to_proto_timezone() {
        use bmc_widget::{ParamDefinition, ParamKind};
        use web::manifest_param_definition::Kind;
        let p = ParamDefinition {
            name: "Timezone".into(),
            description: None,
            is_optional: false,
            kind: ParamKind::Timezone {
                default_value: Some("Europe/Prague".into()),
            },
        };
        let proto = param_definition_to_proto("tz", &p);
        assert_eq!(proto.key, "tz");
        let Some(Kind::ParamTimezone(pt)) = proto.kind else {
            panic!("BUG: expected param_timezone arm");
        };
        assert_eq!(pt.default_value.as_deref(), Some("Europe/Prague"));
    }
}
