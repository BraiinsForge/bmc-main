// Copyright (C) 2025  Braiins Systems s.r.o.

use std::collections::BTreeMap;
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
use tonic::{Code, Request, Response, Status};
use tonic_types::{ErrorDetails, StatusExt};
use uuid::Uuid;

use crate::config::ConfigHandle;
use crate::data::{SceneCycling, SceneCyclingTransition};
use crate::scene;
use crate::web::grpc::GrpcError;
use crate::web::grpc::shared::FieldViolations;
use crate::widget::{Coordinator, WidgetRegistry};

/// Wrap a non-empty FieldViolations into InvalidArgument + BadRequest details.
fn bad_request_status(violations: FieldViolations) -> Option<Status> {
    if violations.is_empty() {
        return None;
    }
    Some(Status::with_error_details(
        Code::InvalidArgument,
        GrpcError::BadRequest.to_string(),
        ErrorDetails::with_bad_request(violations),
    ))
}

fn parse_uuid_field(s: &str, path: &str, violations: &mut FieldViolations) -> Option<Uuid> {
    if let Ok(uuid) = Uuid::parse_str(s) {
        Some(uuid)
    } else {
        violations.push(path.to_owned(), format!("invalid UUID: {s:?}"));
        None
    }
}

fn parse_grid_axis(value: u32, path: &str, violations: &mut FieldViolations) -> Option<u8> {
    if let Ok(v) = u8::try_from(value) {
        Some(v)
    } else {
        violations.push(path.to_owned(), format!("{value} does not fit in u8"));
        None
    }
}

fn parse_widget_size(
    size: i32,
    path: &str,
    violations: &mut FieldViolations,
) -> Option<scene::WidgetSize> {
    match web::WidgetSize::try_from(size) {
        Ok(web::WidgetSize::Small) => Some(scene::WidgetSize::Small),
        Ok(web::WidgetSize::Medium) => Some(scene::WidgetSize::Medium),
        Ok(web::WidgetSize::Large) => Some(scene::WidgetSize::Large),
        Ok(web::WidgetSize::Full) => Some(scene::WidgetSize::Full),
        Ok(web::WidgetSize::Unspecified) => {
            violations.push(
                path.to_owned(),
                "must be one of {Small, Medium, Large, Full}",
            );
            None
        }
        Err(_) => {
            violations.push(
                path.to_owned(),
                format!("must be one of {{Small, Medium, Large, Full}} (got {size})"),
            );
            None
        }
    }
}

struct ParsedAddFullscreenShape {
    config: web::WidgetConfig,
    widget_uid: Uuid,
}

fn parse_add_fullscreen_shape(
    req: web::AddFullscreenSceneRequest,
) -> Result<ParsedAddFullscreenShape, Status> {
    let mut shape = FieldViolations::new();
    let config = req.config.or_else(|| {
        shape.push("config", "config is required");
        None
    });
    let widget_uid = config
        .as_ref()
        .and_then(|c| parse_uuid_field(&c.widget_uid, "config.widget_uid", &mut shape));
    if let Some(status) = bad_request_status(shape) {
        return Err(status);
    }

    match (config, widget_uid) {
        (Some(config), Some(widget_uid)) => Ok(ParsedAddFullscreenShape { config, widget_uid }),
        _ => Err(Status::internal(
            "BUG: shape parsing succeeded with missing required values",
        )),
    }
}

struct ParsedAddWidgetShape {
    scene_id: Uuid,
    position: scene::WidgetPosition,
    size: scene::WidgetSize,
    config_req: web::WidgetConfig,
    widget_uid: Uuid,
}

fn parse_add_widget_shape(req: web::AddWidgetRequest) -> Result<ParsedAddWidgetShape, Status> {
    let mut shape = FieldViolations::new();

    let scene_id = parse_uuid_field(&req.scene_id, "scene_id", &mut shape);
    let proto_pos = req.position.unwrap_or_default();
    let row = parse_grid_axis(proto_pos.row, "position.row", &mut shape);
    let col = parse_grid_axis(proto_pos.col, "position.col", &mut shape);
    let size = parse_widget_size(req.size, "size", &mut shape);
    let config_req = req.config.or_else(|| {
        shape.push("config", "config is required");
        None
    });
    let widget_uid = config_req
        .as_ref()
        .and_then(|c| parse_uuid_field(&c.widget_uid, "config.widget_uid", &mut shape));

    if let Some(status) = bad_request_status(shape) {
        return Err(status);
    }

    match (scene_id, row, col, size, config_req, widget_uid) {
        (Some(scene_id), Some(row), Some(col), Some(size), Some(config_req), Some(widget_uid)) => {
            Ok(ParsedAddWidgetShape {
                scene_id,
                position: scene::WidgetPosition { row, col },
                size,
                config_req,
                widget_uid,
            })
        }
        _ => Err(Status::internal(
            "BUG: shape parsing succeeded with missing required values",
        )),
    }
}

struct ParsedUpdateWidgetShape {
    scene_id: Uuid,
    widget_id: Uuid,
    proto_position: web::WidgetPosition,
    position: scene::WidgetPosition,
    size: scene::WidgetSize,
    params: Option<web::WidgetDataStruct>,
}

fn parse_update_widget_shape(
    req: web::UpdateWidgetRequest,
) -> Result<ParsedUpdateWidgetShape, Status> {
    let mut shape = FieldViolations::new();

    let scene_id = parse_uuid_field(&req.scene_id, "scene_id", &mut shape);
    let widget_id = parse_uuid_field(&req.id, "id", &mut shape);
    let proto_position = req.position.unwrap_or_default();
    let row = parse_grid_axis(proto_position.row, "position.row", &mut shape);
    let col = parse_grid_axis(proto_position.col, "position.col", &mut shape);
    let size = parse_widget_size(req.size, "size", &mut shape);

    if let Some(status) = bad_request_status(shape) {
        return Err(status);
    }

    match (scene_id, widget_id, row, col, size) {
        (Some(scene_id), Some(widget_id), Some(row), Some(col), Some(size)) => {
            Ok(ParsedUpdateWidgetShape {
                scene_id,
                widget_id,
                proto_position,
                position: scene::WidgetPosition { row, col },
                size,
                params: req.params,
            })
        }
        _ => Err(Status::internal(
            "BUG: shape parsing succeeded with missing required values",
        )),
    }
}

pub(crate) struct SceneManagementService {
    widget_registry: Arc<WidgetRegistry>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    coordinator: Arc<Coordinator>,
    /// Scene currently held open by a `preview_scene` stream. While set,
    /// that scene overrides the first-enabled pick in `restore_active_scene`
    /// so edits made during a preview stay focused on it.
    ///
    /// **Lock order:** always acquire this mutex *before* `config_handle`.
    /// `PreviewGuard::drop` and every method on this service follows that
    /// order; without it tokio's write-preferring `RwLock` can deadlock
    /// under a queued config writer.
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
        let preview_id = *self.preview_scene_id.lock().await;
        let config = self.config_handle.read().await;
        self.coordinator.refresh_scene_cycling(&config.scenes);
        if let Some(preview_id) = preview_id
            && let Some(scene) = config.scenes.get(&preview_id)
        {
            self.coordinator.set_active_scene(scene);
        }
    }

    /// Save config, returning a gRPC-friendly error on failure.
    async fn save_config(config: &mut ConfigHandle) -> Result<(), Status> {
        config
            .save()
            .await
            .map_err(|e| Status::internal(format!("failed to save config: {e}")))
    }
}

/// Build a typed param map ready for storage. Caller MUST have already
/// passed `overrides` through `validate_widget_params` for the same
/// manifest — this function panics on a wire-shape / declared-kind
/// mismatch, since validation should have caught it.
///
/// Manifest keys missing from `overrides` are filled with the manifest's
/// default value (or `Null` for optional params without a default).
pub(crate) fn build_widget_params(
    manifest: &bmc_widget::Manifest,
    overrides: &web::WidgetDataStruct,
) -> BTreeMap<bmc_widget::ParamKey, bmc_widget::ParamValue> {
    let mut out = BTreeMap::new();
    for (key, def) in &manifest.params {
        let value = match overrides.fields.get(key.as_str()) {
            Some(wdv) => wire_to_param_value(wdv, &def.kind),
            None => bmc_widget::ParamValue::from_param_kind_default(&def.kind),
        };
        out.insert(key.clone(), value);
    }
    out
}

fn wire_to_param_value(
    wdv: &web::WidgetDataValue,
    param_kind: &ParamKind,
) -> bmc_widget::ParamValue {
    use bmc_widget::ParamValue as PV;
    use web::widget_data_value::Kind as VK;
    let Some(kind) = wdv.kind.as_ref() else {
        // Missing kind treated as null on the wire; validators that
        // want to reject this set is_optional=false. Either way, store
        // null and let the manifest semantics drive behavior.
        return PV::Null;
    };
    match (param_kind, kind) {
        (_, VK::NullValue(())) => PV::Null,
        (ParamKind::Boolean { .. }, VK::BooleanValue(b)) => PV::Boolean(*b),
        (ParamKind::Integer { .. }, VK::IntegerValue(i)) => PV::Integer(*i),
        (ParamKind::Double { .. }, VK::DoubleValue(d)) => PV::Double(*d),
        (ParamKind::String { .. } | ParamKind::Timezone { .. }, VK::StringValue(s)) => {
            PV::String(s.clone())
        }
        _ => panic!("BUG: validate_widget_params must reject param/value kind mismatch"),
    }
}

fn params_to_widget_data_struct(
    params: &BTreeMap<bmc_widget::ParamKey, bmc_widget::ParamValue>,
) -> web::WidgetDataStruct {
    web::WidgetDataStruct {
        fields: params
            .iter()
            .map(|(k, v)| (k.as_str().to_owned(), param_value_to_wire(v)))
            .collect(),
    }
}

fn param_value_to_wire(v: &bmc_widget::ParamValue) -> web::WidgetDataValue {
    use bmc_widget::ParamValue as PV;
    use web::widget_data_value::Kind as VK;
    let arm = match v {
        PV::Null => VK::NullValue(()),
        PV::Boolean(b) => VK::BooleanValue(*b),
        PV::Integer(i) => VK::IntegerValue(*i),
        PV::Double(d) => VK::DoubleValue(*d),
        PV::String(s) => VK::StringValue(s.clone()),
    };
    web::WidgetDataValue { kind: Some(arm) }
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
) -> FieldViolations {
    use web::widget_data_value::Kind as VK;
    let mut violations = FieldViolations::new();

    if matches!(mode, ValidateMode::Update) {
        for key in manifest.params.keys() {
            if !params.fields.contains_key(key.as_str()) {
                violations.push(
                    format!(r#"params["{}"]"#, key.as_str()),
                    "Value is required",
                );
            }
        }
    }

    for (key, value) in &params.fields {
        let path = format!(r#"params["{key}"]"#);
        let Some(def) = manifest.params.get(key.as_str()) else {
            violations.push(path, "Unknown param");
            continue;
        };
        let Some(kind) = value.kind.as_ref() else {
            violations.push(path, "WidgetDataValue.kind unset");
            continue;
        };

        if matches!(kind, VK::NullValue(())) {
            if !def.is_optional {
                violations.push(path, "Value is required");
            }
            continue;
        }

        validate_widget_param_value(&path, &def.kind, kind, &mut violations);
    }
    violations
}

fn type_mismatch_message(kind: &ParamKind) -> &'static str {
    match kind {
        ParamKind::String { .. } => "Must be text",
        ParamKind::Integer { .. } => "Must be a whole number",
        ParamKind::Double { .. } => "Must be a number",
        ParamKind::Boolean { .. } => "Must be true or false",
        ParamKind::Timezone { .. } => "Must be a timezone",
    }
}

fn validate_widget_param_value(
    path: &str,
    param_kind: &ParamKind,
    kind: &web::widget_data_value::Kind,
    violations: &mut FieldViolations,
) {
    use web::widget_data_value::Kind as VK;
    match (param_kind, kind) {
        (ParamKind::String { enum_values, .. }, VK::StringValue(s)) => {
            if !enum_values.is_empty() && !enum_values.iter().any(|o| &o.value == s) {
                violations.push(path.to_owned(), "Must be one of the listed options");
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
                violations.push(path.to_owned(), format!("Must be at least {lo}"));
            }
            if let Some(hi) = max
                && i > hi
            {
                violations.push(path.to_owned(), format!("Must be at most {hi}"));
            }
            if !enum_values.is_empty() && !enum_values.iter().any(|o| o.value == *i) {
                violations.push(path.to_owned(), "Must be one of the listed options");
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
                violations.push(path.to_owned(), "Must be a finite number");
                return;
            }
            if let Some(lo) = min
                && d < lo
            {
                violations.push(path.to_owned(), format!("Must be at least {lo}"));
            }
            if let Some(hi) = max
                && d > hi
            {
                violations.push(path.to_owned(), format!("Must be at most {hi}"));
            }
            if !enum_values.is_empty()
                && !enum_values.iter().any(|o| {
                    bmc_widget::f64_canonical_bits(o.value) == bmc_widget::f64_canonical_bits(*d)
                })
            {
                violations.push(path.to_owned(), "Must be one of the listed options");
            }
        }
        (other_kind, _) => {
            violations.push(path.to_owned(), type_mismatch_message(other_kind));
        }
    }
}

fn scene_widget_to_proto(widget: &scene::Widget) -> web::Widget {
    web::Widget {
        id: widget.id.to_string(),
        position: Some(web::WidgetPosition {
            row: u32::from(widget.position.row),
            col: u32::from(widget.position.col),
        }),
        size: size_type_to_proto(widget.size.into()),
        config: Some(web::WidgetConfig {
            widget_uid: widget.widget_type_id.to_string(),
            params: Some(params_to_widget_data_struct(&widget.params)),
        }),
    }
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

fn parse_scene_cycling_transition(value: i32) -> Result<SceneCyclingTransition, Status> {
    match web::SceneCyclingTransition::try_from(value) {
        Ok(web::SceneCyclingTransition::Slide) => Ok(SceneCyclingTransition::Slide),
        Ok(web::SceneCyclingTransition::Fade) => Ok(SceneCyclingTransition::Fade),
        Ok(web::SceneCyclingTransition::Unspecified) => Err(Status::invalid_argument(
            "scene_cycling.transition must be Slide or Fade (got Unspecified)",
        )),
        Err(_) => Err(Status::invalid_argument(format!(
            "scene_cycling.transition: unknown value {value}"
        ))),
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
        let ParsedAddFullscreenShape { config, widget_uid } =
            parse_add_fullscreen_shape(request.into_inner())?;

        let info = self
            .widget_registry
            .get(&widget_uid)
            .ok_or_else(|| Status::failed_precondition("widget manifest not installed"))?;
        let manifest = &info.manifest;

        if !self
            .widget_registry
            .supports_size(&widget_uid, SizeType::Full)
        {
            return Err(Status::failed_precondition(format!(
                "widget {widget_uid} does not support size {:?}",
                SizeType::Full,
            )));
        }

        let params = config.params.unwrap_or_default();
        if let Some(status) =
            bad_request_status(validate_widget_params(manifest, &params, ValidateMode::Add))
        {
            return Err(status);
        }
        let typed_params = build_widget_params(manifest, &params);

        let scene = scene::Scene::fullscreen(widget_uid, typed_params);
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

        let scene_id_key = scene::SceneId::from(id);
        let was_enabled;
        {
            let mut config = self.config_handle.write().await;
            let scene = config
                .scenes
                .get_mut(&scene_id_key)
                .ok_or_else(|| Status::not_found(format!("scene not found: {id}")))?;

            if req.cycle_duration_sec == Some(0) {
                return Err(Status::invalid_argument(
                    "cycle_duration_sec must be >= 1 when set",
                ));
            }

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

        let scene_was_disabled = {
            let mut preview = self.preview_scene_id.lock().await;
            if preview.is_some() {
                return Err(Status::resource_exhausted("scene preview already active"));
            }

            let config = self.config_handle.read().await;
            let scene = config
                .scenes
                .get(&scene_id)
                .ok_or_else(|| Status::not_found(format!("scene not found: {scene_id}")))?;

            *preview = Some(scene_id);

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
            transition: parse_scene_cycling_transition(cycling.transition)?,
        });

        Self::save_config(&mut config).await?;

        Ok(Response::new(()))
    }

    async fn add_widget(
        &self,
        request: Request<web::AddWidgetRequest>,
    ) -> Result<Response<String>, Status> {
        let ParsedAddWidgetShape {
            scene_id,
            position,
            size,
            config_req,
            widget_uid,
        } = parse_add_widget_shape(request.into_inner())?;

        let info = self
            .widget_registry
            .get(&widget_uid)
            .ok_or_else(|| Status::failed_precondition("widget manifest not installed"))?;
        let manifest = &info.manifest;

        if !self.widget_registry.supports_size(&widget_uid, size.into()) {
            return Err(Status::failed_precondition(format!(
                "widget {widget_uid} does not support size {size}",
            )));
        }

        let params = config_req.params.unwrap_or_default();
        if let Some(status) =
            bad_request_status(validate_widget_params(manifest, &params, ValidateMode::Add))
        {
            return Err(status);
        }
        let typed_params = build_widget_params(manifest, &params);

        let widget = scene::Widget::new(widget_uid, typed_params, position, size);
        let widget_id = widget.id.to_string();

        let scene_id_key = scene::SceneId::from(scene_id);

        let preview_snapshot = *self.preview_scene_id.lock().await;

        let widget_to_spawn = {
            let mut config = self.config_handle.write().await;
            let scene = config
                .scenes
                .get_mut(&scene_id_key)
                .ok_or_else(|| Status::not_found(format!("scene not found: {scene_id}")))?;

            validate_widget_placement(scene, &widget, None)?;

            let showing = scene.enabled || preview_snapshot == Some(scene_id_key);
            scene.widgets.insert(widget.id, widget.clone());
            Self::save_config(&mut config).await?;

            if showing { Some(widget) } else { None }
        };

        if let Some(widget) = widget_to_spawn {
            self.coordinator.spawn_widget(&scene_id_key, &widget).await;
        }
        self.restore_active_scene().await;

        Ok(Response::new(widget_id))
    }

    async fn update_widget(
        &self,
        request: Request<web::UpdateWidgetRequest>,
    ) -> Result<Response<()>, Status> {
        let ParsedUpdateWidgetShape {
            scene_id,
            widget_id,
            proto_position,
            position,
            size,
            params,
        } = parse_update_widget_shape(request.into_inner())?;

        let scene_id_key = scene::SceneId::from(scene_id);
        let widget_id_key = scene::WidgetId::from(widget_id);
        let preview_snapshot = *self.preview_scene_id.lock().await;

        let widget_snapshot = {
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

            if !self.widget_registry.supports_size(&widget_uid, size.into()) {
                return Err(Status::failed_precondition(format!(
                    "widget {widget_uid} does not support size {size}",
                )));
            }

            let params = params.unwrap_or_default();
            if let Some(status) = bad_request_status(validate_widget_params(
                manifest,
                &params,
                ValidateMode::Update,
            )) {
                return Err(status);
            }
            let typed_params = build_widget_params(manifest, &params);

            // Build the post-update widget snapshot first so placement
            // validation runs against immutable state. If anything below
            // fails the in-memory ConfigHandle is left untouched.
            let existing = scene
                .widgets
                .get(&widget_id_key)
                .expect("BUG: widget was just found above");
            let updated_widget = scene::Widget {
                id: existing.id,
                position,
                size,
                widget_type_id: existing.widget_type_id,
                params: typed_params,
            };
            validate_widget_placement(scene, &updated_widget, Some(widget_id_key))?;

            scene
                .widgets
                .insert(updated_widget.id, updated_widget.clone());

            let showing = scene.enabled || preview_snapshot == Some(scene_id_key);
            Self::save_config(&mut config).await?;
            (updated_widget, showing)
        };

        let (widget_snapshot, showing) = widget_snapshot;

        if showing {
            let instance_id = widget_snapshot.id.as_uuid().to_string();
            self.coordinator.stop_widget(&instance_id).await;
            self.coordinator
                .spawn_widget(&scene_id_key, &widget_snapshot)
                .await;
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
        use bmc_widget::ParamValue as PV;
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
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved.get("name"), Some(&PV::String("hello".into())));
    }

    #[test]
    fn build_widget_params_seeds_optional_no_default_with_null() {
        use bmc_widget::ParamValue as PV;
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
        assert_eq!(resolved.get("name"), Some(&PV::Null));
    }

    #[test]
    fn build_widget_params_override_wins() {
        use bmc_widget::ParamValue as PV;
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
        assert_eq!(resolved.get("name"), Some(&PV::String("world".into())));
    }

    #[test]
    fn build_widget_params_seeds_each_kind_with_default() {
        use bmc_widget::ParamValue as PV;
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
        assert_eq!(resolved.len(), 5);
        assert_eq!(resolved.get("s"), Some(&PV::String("x".into())));
        assert_eq!(resolved.get("i"), Some(&PV::Integer(7)));
        assert_eq!(resolved.get("d"), Some(&PV::Double(2.5)));
        assert_eq!(resolved.get("b"), Some(&PV::Boolean(true)));
        assert_eq!(resolved.get("t"), Some(&PV::String("UTC".into())));
    }

    #[test]
    fn params_to_widget_data_struct_round_trips_each_arm() {
        use bmc_widget::{ParamKey, ParamValue as PV};
        use web::widget_data_value::Kind as VK;
        let key = |k: &str| ParamKey::try_new(k.to_owned()).expect("BUG: valid key");

        let map: BTreeMap<ParamKey, PV> = [
            (key("s"), PV::String("hello".into())),
            (key("i"), PV::Integer(42)),
            (key("d"), PV::Double(2.5)),
            (key("b"), PV::Boolean(false)),
            (key("n"), PV::Null),
        ]
        .into_iter()
        .collect();

        let wire = params_to_widget_data_struct(&map);
        assert!(matches!(wire.fields["s"].kind, Some(VK::StringValue(_))));
        assert!(matches!(wire.fields["i"].kind, Some(VK::IntegerValue(42))));
        assert!(matches!(wire.fields["d"].kind, Some(VK::DoubleValue(_))));
        assert!(matches!(
            wire.fields["b"].kind,
            Some(VK::BooleanValue(false))
        ));
        assert!(matches!(wire.fields["n"].kind, Some(VK::NullValue(()))));
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
            kind: Some(web::widget_data_value::Kind::NullValue(())),
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

    fn violation_count(
        manifest: &bmc_widget::Manifest,
        params: &web::WidgetDataStruct,
        mode: ValidateMode,
    ) -> usize {
        let violations = validate_widget_params(manifest, params, mode);
        let v: Vec<tonic_types::FieldViolation> = violations.into();
        v.len()
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
        assert!(validate_widget_params(&manifest, &params, ValidateMode::Add).is_empty());
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
        assert_eq!(violation_count(&manifest, &params, ValidateMode::Add), 1);
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
        assert_eq!(violation_count(&manifest, &params, ValidateMode::Add), 1);
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
        assert_eq!(violation_count(&manifest, &params, ValidateMode::Add), 1);
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
        assert_eq!(violation_count(&manifest, &params, ValidateMode::Add), 1);
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
        assert_eq!(violation_count(&manifest, &params, ValidateMode::Add), 1);
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
        assert!(validate_widget_params(&manifest, &params, ValidateMode::Add).is_empty());
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
        assert_eq!(violation_count(&manifest, &params, ValidateMode::Add), 1);
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
        assert_eq!(violation_count(&manifest, &params, ValidateMode::Add), 1);
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
        assert_eq!(violation_count(&manifest, &params, ValidateMode::Add), 1);
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
        assert_eq!(violation_count(&manifest, &params, ValidateMode::Add), 1);
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
        assert_eq!(violation_count(&manifest, &params, ValidateMode::Add), 1);
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
        assert_eq!(violation_count(&manifest, &params, ValidateMode::Add), 1);
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
        assert!(validate_widget_params(&manifest, &params, ValidateMode::Add).is_empty());
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
        assert_eq!(violation_count(&manifest, &params, ValidateMode::Add), 1);
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
        assert_eq!(violation_count(&manifest, &params, ValidateMode::Update), 1);
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
        assert!(validate_widget_params(&manifest, &params, ValidateMode::Add).is_empty());
    }

    #[test]
    fn validate_widget_params_accumulates_per_field_violations() {
        let manifest = manifest_with_params(&[
            (
                "n",
                bmc_widget::ParamKind::Integer {
                    min: Some(0),
                    max: Some(10),
                    step: None,
                    enum_values: vec![],
                    default_value: Some(5),
                },
                false,
            ),
            (
                "color",
                bmc_widget::ParamKind::String {
                    format: None,
                    enum_values: vec![bmc_widget::StringOption {
                        value: "red".into(),
                        label: "Red".into(),
                    }],
                    default_value: Some("red".into()),
                },
                false,
            ),
        ]);
        let params = web::WidgetDataStruct {
            fields: [
                ("n".to_owned(), wdv_integer(99)),
                ("color".to_owned(), wdv_string("blue")),
            ]
            .into_iter()
            .collect(),
        };

        let violations = validate_widget_params(&manifest, &params, ValidateMode::Add);
        let v: Vec<tonic_types::FieldViolation> = violations.into();
        assert_eq!(v.len(), 2, "both broken fields must be reported");

        let fields: std::collections::HashSet<&str> = v.iter().map(|x| x.field.as_str()).collect();
        assert!(fields.contains(r#"params["n"]"#));
        assert!(fields.contains(r#"params["color"]"#));
    }

    #[test]
    fn add_widget_request_shape_errors_accumulate_in_one_response() {
        let mut v = FieldViolations::new();
        assert!(parse_uuid_field("not-a-uuid", "scene_id", &mut v).is_none());
        assert!(parse_grid_axis(999, "position.row", &mut v).is_none());
        assert!(parse_widget_size(web::WidgetSize::Unspecified.into(), "size", &mut v).is_none());

        let violations: Vec<tonic_types::FieldViolation> = v.into();
        assert_eq!(violations.len(), 3);
        let fields: std::collections::HashSet<&str> =
            violations.iter().map(|x| x.field.as_str()).collect();
        assert!(fields.contains("scene_id"));
        assert!(fields.contains("position.row"));
        assert!(fields.contains("size"));
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

    fn scene_with_widget(
        widget_uid: uuid::Uuid,
        params: BTreeMap<bmc_widget::ParamKey, bmc_widget::ParamValue>,
    ) -> crate::scene::Scene {
        crate::scene::Scene::fullscreen(widget_uid, params)
    }

    fn proto_scene_first_widget(proto: &web::Scene) -> &web::Widget {
        match proto.kind.as_ref().expect("BUG: kind") {
            web::scene::Kind::Fullscreen(f) => f.widget.as_ref().expect("BUG: widget"),
            web::scene::Kind::Combined(c) => c.widgets.first().expect("BUG: widget"),
        }
    }

    #[test]
    fn scene_to_proto_emits_typed_params_directly() {
        use bmc_widget::{ParamKey, ParamValue as PV};
        use web::widget_data_value::Kind as VK;

        let widget_uid = uuid::Uuid::new_v4();
        let key = ParamKey::try_new("x".to_owned()).expect("BUG: valid key");
        let params: BTreeMap<ParamKey, PV> = [(key, PV::Integer(5))].into_iter().collect();

        let scene = scene_with_widget(widget_uid, params);
        let proto = scene_to_proto(&scene);
        let widget = proto_scene_first_widget(&proto);
        let config = widget.config.as_ref().expect("BUG: config");
        let params = config.params.as_ref().expect("BUG: params");
        assert!(matches!(params.fields["x"].kind, Some(VK::IntegerValue(5))));
    }

    #[test]
    fn scene_to_proto_emits_each_param_value_arm() {
        use bmc_widget::{ParamKey, ParamValue as PV};
        use web::widget_data_value::Kind as VK;

        let widget_uid = uuid::Uuid::new_v4();
        let key = |k: &str| ParamKey::try_new(k.to_owned()).expect("BUG: valid key");
        let params: BTreeMap<ParamKey, PV> = [
            (key("s"), PV::String("hi".into())),
            (key("i"), PV::Integer(7)),
            (key("d"), PV::Double(0.25)),
            (key("b"), PV::Boolean(true)),
            (key("n"), PV::Null),
        ]
        .into_iter()
        .collect();

        let scene = scene_with_widget(widget_uid, params);
        let proto = scene_to_proto(&scene);
        let widget = proto_scene_first_widget(&proto);
        let fields = &widget
            .config
            .as_ref()
            .expect("BUG: config")
            .params
            .as_ref()
            .expect("BUG: params")
            .fields;
        assert!(matches!(fields["s"].kind, Some(VK::StringValue(_))));
        assert!(matches!(fields["i"].kind, Some(VK::IntegerValue(7))));
        assert!(matches!(fields["d"].kind, Some(VK::DoubleValue(_))));
        assert!(matches!(fields["b"].kind, Some(VK::BooleanValue(true))));
        assert!(matches!(fields["n"].kind, Some(VK::NullValue(()))));
    }

    #[test]
    fn set_scene_cycling_rejects_unspecified_transition() {
        let req = web::SetSceneCyclingRequest {
            scene_cycling: Some(web::SceneCycling {
                automatic_cycling_enabled: false,
                automatic_cycling_default_duration_sec: 30,
                transition: web::SceneCyclingTransition::Unspecified.into(),
            }),
        };
        let err = parse_scene_cycling_transition(
            req.scene_cycling
                .as_ref()
                .expect("BUG: cycling set above")
                .transition,
        )
        .expect_err("BUG: must reject unspecified transition");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().contains("transition"),
            "message should name the field: {err:?}"
        );
    }

    #[test]
    fn set_scene_cycling_rejects_unknown_transition_int() {
        let err = parse_scene_cycling_transition(9999)
            .expect_err("BUG: must reject unknown transition int");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn set_scene_cycling_accepts_slide_and_fade() {
        assert_eq!(
            parse_scene_cycling_transition(web::SceneCyclingTransition::Slide.into())
                .expect("BUG: Slide must parse"),
            SceneCyclingTransition::Slide,
        );
        assert_eq!(
            parse_scene_cycling_transition(web::SceneCyclingTransition::Fade.into())
                .expect("BUG: Fade must parse"),
            SceneCyclingTransition::Fade,
        );
    }

    #[test]
    fn parse_widget_size_unspecified_message_names_variants() {
        let mut v = FieldViolations::new();
        let result = parse_widget_size(web::WidgetSize::Unspecified.into(), "size", &mut v);
        assert!(result.is_none());
        let violations: Vec<tonic_types::FieldViolation> = v.into();
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].field, "size");
        let msg = &violations[0].description;
        assert!(
            msg.contains("Small")
                && msg.contains("Medium")
                && msg.contains("Large")
                && msg.contains("Full"),
            "message should name the four valid variants, got: {msg}",
        );
    }

    #[test]
    fn parse_widget_size_unknown_int_message_includes_value() {
        let mut v = FieldViolations::new();
        let result = parse_widget_size(9999, "size", &mut v);
        assert!(result.is_none());
        let violations: Vec<tonic_types::FieldViolation> = v.into();
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].field, "size");
        let msg = &violations[0].description;
        assert!(
            msg.contains("9999"),
            "message should include the bad value, got: {msg}"
        );
    }

    fn first_violation_desc(
        manifest: &bmc_widget::Manifest,
        params: &web::WidgetDataStruct,
    ) -> String {
        let v: Vec<tonic_types::FieldViolation> =
            validate_widget_params(manifest, params, ValidateMode::Add).into();
        assert_eq!(v.len(), 1, "expected exactly one violation");
        v.into_iter()
            .next()
            .expect("BUG: vec checked non-empty")
            .description
    }

    #[test]
    fn validate_widget_params_message_string_type_mismatch() {
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
        assert_eq!(first_violation_desc(&manifest, &params), "Must be text");
    }

    #[test]
    fn validate_widget_params_message_integer_type_mismatch() {
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
        let params = fields_one("count", wdv_string("abc"));
        assert_eq!(
            first_violation_desc(&manifest, &params),
            "Must be a whole number",
        );
    }

    #[test]
    fn validate_widget_params_message_double_type_mismatch() {
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
        let params = fields_one("ratio", wdv_string("abc"));
        assert_eq!(first_violation_desc(&manifest, &params), "Must be a number",);
    }

    #[test]
    fn validate_widget_params_message_boolean_type_mismatch() {
        let manifest = single_param_manifest(
            "flag",
            bmc_widget::ParamKind::Boolean {
                default_value: Some(false),
            },
            false,
        );
        let params = fields_one("flag", wdv_string("yes"));
        assert_eq!(
            first_violation_desc(&manifest, &params),
            "Must be true or false",
        );
    }

    #[test]
    fn validate_widget_params_message_timezone_type_mismatch() {
        let manifest = single_param_manifest(
            "tz",
            bmc_widget::ParamKind::Timezone {
                default_value: None,
            },
            true,
        );
        let params = fields_one("tz", wdv_integer(0));
        assert_eq!(
            first_violation_desc(&manifest, &params),
            "Must be a timezone",
        );
    }

    #[test]
    fn validate_widget_params_message_integer_below_min() {
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
        assert_eq!(
            first_violation_desc(&manifest, &params),
            "Must be at least 5",
        );
    }

    #[test]
    fn validate_widget_params_message_integer_above_max() {
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
        assert_eq!(
            first_violation_desc(&manifest, &params),
            "Must be at most 10",
        );
    }

    #[test]
    fn validate_widget_params_message_double_not_finite() {
        let manifest = single_param_manifest(
            "v",
            bmc_widget::ParamKind::Double {
                min: None,
                max: None,
                step: None,
                enum_values: vec![],
                default_value: Some(1.0),
            },
            false,
        );
        let params = fields_one("v", wdv_double(f64::NAN));
        assert_eq!(
            first_violation_desc(&manifest, &params),
            "Must be a finite number",
        );
    }

    #[test]
    fn validate_widget_params_message_double_below_min() {
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
        assert_eq!(
            first_violation_desc(&manifest, &params),
            "Must be at least 0",
        );
    }

    #[test]
    fn validate_widget_params_message_double_above_max() {
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
        let params = fields_one("ratio", wdv_double(1.5));
        assert_eq!(
            first_violation_desc(&manifest, &params),
            "Must be at most 1",
        );
    }

    #[test]
    fn validate_widget_params_message_enum_string_mismatch() {
        let manifest = single_param_manifest(
            "style",
            bmc_widget::ParamKind::String {
                format: None,
                enum_values: vec![bmc_widget::StringOption {
                    value: "dark".into(),
                    label: "Dark".into(),
                }],
                default_value: Some("dark".into()),
            },
            false,
        );
        let params = fields_one("style", wdv_string("solarized"));
        assert_eq!(
            first_violation_desc(&manifest, &params),
            "Must be one of the listed options",
        );
    }

    #[test]
    fn validate_widget_params_message_enum_integer_mismatch() {
        let manifest = single_param_manifest(
            "level",
            bmc_widget::ParamKind::Integer {
                min: None,
                max: None,
                step: None,
                enum_values: vec![bmc_widget::IntegerOption {
                    value: 1,
                    label: "One".into(),
                }],
                default_value: Some(1),
            },
            false,
        );
        let params = fields_one("level", wdv_integer(99));
        assert_eq!(
            first_violation_desc(&manifest, &params),
            "Must be one of the listed options",
        );
    }
}
