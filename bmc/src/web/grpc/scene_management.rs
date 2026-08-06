// Copyright (C) 2025  Braiins Systems s.r.o.
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

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr as _;
use std::sync::Arc;
use std::time::Duration;

use bmc_grpc::web;
use bmc_grpc::web::scene_management_service_server::SceneManagementService as GrpcSceneManagementService;
use bmc_shared_time::time::Timezone;
use bmc_widget_manifest::{CredentialKey, ParamDefinition, ParamKind};
use futures::stream::{BoxStream, StreamExt};
use indexmap::IndexMap;
use tokio::sync::{Mutex, RwLock};
use tokio::time;
use tokio_stream::wrappers::IntervalStream;
use tonic::{Code, Request, Response, Status};
use tonic_types::{ErrorDetails, StatusExt};
use uuid::Uuid;

use bmc_platform::HardwareCapabilities;

use crate::compositor::WidgetInstanceKey;
use crate::config::ConfigHandle;
use crate::credential;
use crate::data::{Account, AccountId, SceneCycling, SceneCyclingTransition};
use crate::led_coordinator::{Layer, LedCoordinatorHandle};
use crate::scene;
use crate::secret_store::SecretStoreHandle;
use crate::web::grpc::GrpcError;
use crate::web::grpc::shared::FieldViolations;
use crate::widget::coordinator::{PendingWidgetRegistration, WidgetStopBatch};
use crate::widget::{Coordinator, WidgetRegistry};

pub(crate) struct PlatformDescriptor {
    fullscreen: crate::widget::ViewportDescriptor,
    slot_sizes: Vec<(web::WidgetSize, crate::widget::ViewportDescriptor)>,
}

fn slot_size_descriptors() -> Vec<(web::WidgetSize, crate::widget::ViewportDescriptor)> {
    [
        web::WidgetSize::Small,
        web::WidgetSize::Medium,
        web::WidgetSize::Large,
    ]
    .into_iter()
    .map(|size| {
        let descriptor = match scene::WidgetPlacement::try_from(size) {
            Ok(scene::WidgetPlacement::SlotSpan(span)) => {
                crate::widget::slot_span_descriptor(span.columns, span.rows)
            }
            Ok(scene::WidgetPlacement::Fullscreen) | Err(UnsupportedWidgetSize) => None,
        }
        .expect("BUG: slot size labels resolve to slot span descriptors");
        (size, descriptor)
    })
    .collect()
}

#[cfg(test)]
fn bmc100_platform_descriptor() -> PlatformDescriptor {
    PlatformDescriptor::from(
        &bmc_platform::HardwareProfile::for_product(bmc_platform::Product::Bmc100).capabilities(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unsupported widget size label")]
pub struct UnsupportedWidgetSize;

impl TryFrom<web::WidgetSize> for scene::WidgetPlacement {
    type Error = UnsupportedWidgetSize;

    fn try_from(size: web::WidgetSize) -> Result<Self, Self::Error> {
        match size {
            web::WidgetSize::Small => Ok(Self::SlotSpan(scene::SlotSpan {
                columns: 1,
                rows: 1,
            })),
            web::WidgetSize::Medium => Ok(Self::SlotSpan(scene::SlotSpan {
                columns: 2,
                rows: 1,
            })),
            web::WidgetSize::Large => Ok(Self::SlotSpan(scene::SlotSpan {
                columns: 2,
                rows: 2,
            })),
            web::WidgetSize::Full => Ok(Self::Fullscreen),
            web::WidgetSize::Unspecified => Err(UnsupportedWidgetSize),
        }
    }
}

impl TryFrom<&scene::WidgetPlacement> for web::WidgetSize {
    type Error = UnsupportedWidgetSize;

    fn try_from(placement: &scene::WidgetPlacement) -> Result<Self, Self::Error> {
        match placement {
            scene::WidgetPlacement::Fullscreen => Ok(Self::Full),
            scene::WidgetPlacement::SlotSpan(s) => match (s.columns, s.rows) {
                (1, 1) => Ok(Self::Small),
                (2, 1) => Ok(Self::Medium),
                (2, 2) => Ok(Self::Large),
                _ => Err(UnsupportedWidgetSize),
            },
        }
    }
}

impl PlatformDescriptor {
    fn descriptor_for_size(
        &self,
        size: web::WidgetSize,
    ) -> Option<crate::widget::ViewportDescriptor> {
        if size == web::WidgetSize::Full {
            return Some(self.fullscreen);
        }
        self.slot_sizes
            .iter()
            .find_map(|(label, descriptor)| (*label == size).then_some(*descriptor))
    }

    fn descriptor_for_placement(
        &self,
        placement: &scene::WidgetPlacement,
    ) -> Option<crate::widget::ViewportDescriptor> {
        web::WidgetSize::try_from(placement)
            .ok()
            .and_then(|size| self.descriptor_for_size(size))
    }
}

impl From<&HardwareCapabilities> for PlatformDescriptor {
    fn from(caps: &HardwareCapabilities) -> Self {
        let fullscreen = crate::widget::ViewportDescriptor {
            viewport_shape: widget_viewport_shape_from_caps(caps),
            width: caps.display.width,
            height: caps.display.height,
            dpi: caps.display.dpi,
        };
        let slot_sizes = if caps.slot_grid.is_some() {
            slot_size_descriptors()
        } else {
            Vec::new()
        };
        Self {
            fullscreen,
            slot_sizes,
        }
    }
}

fn reject_combined_when_no_slot_grid(caps: &HardwareCapabilities) -> Result<(), Status> {
    if caps.slot_grid.is_none() {
        return Err(Status::failed_precondition(
            "combined scenes are not supported on this hardware",
        ));
    }
    Ok(())
}

fn widget_viewport_shape_from_caps(
    caps: &HardwareCapabilities,
) -> bmc_widget_manifest::ViewportShape {
    use crate::compositor::DisplayShape as Caps;
    use bmc_widget_manifest::ViewportShape as Manifest;
    match caps.display.shape {
        Caps::Rectangular => Manifest::Rectangular,
        Caps::Round => Manifest::Round,
    }
}

fn stamp_widget_viewport_shape_from_caps(widget: &mut scene::Widget, caps: &HardwareCapabilities) {
    widget.viewport_shape = widget_viewport_shape_from_caps(caps);
}

pub(crate) fn supported_sizes_for_constraints(
    platform: &PlatformDescriptor,
    constraints: &[bmc_widget_manifest::WidgetViewportConstraint],
) -> Vec<web::WidgetSize> {
    platform
        .slot_sizes
        .iter()
        .filter_map(|(size, descriptor)| {
            constraints
                .iter()
                .any(|c| descriptor.matched_by(c))
                .then_some(*size)
        })
        .chain(
            constraints
                .iter()
                .any(|c| platform.fullscreen.matched_by(c))
                .then_some(web::WidgetSize::Full),
        )
        .collect()
}

/// Wrap FieldViolations into InvalidArgument + BadRequest details. Callers
/// must check `is_empty()` themselves — this builds the status unconditionally.
fn bad_request_status(violations: FieldViolations) -> Status {
    Status::with_error_details(
        Code::InvalidArgument,
        GrpcError::BadRequest.to_string(),
        ErrorDetails::with_bad_request(violations),
    )
}

fn live_params_status(error: &crate::compositor::CompositorError) -> Status {
    Status::internal(format!("failed to push live params to widget: {error}"))
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
) -> Option<scene::WidgetPlacement> {
    match web::WidgetSize::try_from(size) {
        Ok(web::WidgetSize::Unspecified) => {
            violations.push(
                path.to_owned(),
                "must be one of {Small, Medium, Large, Full}",
            );
            None
        }
        Ok(web_size) => scene::WidgetPlacement::try_from(web_size).ok(),
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
    if !shape.is_empty() {
        return Err(bad_request_status(shape));
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
    placement: scene::WidgetPlacement,
    config_req: web::WidgetConfig,
    widget_uid: Uuid,
}

fn parse_add_widget_shape(req: web::AddWidgetRequest) -> Result<ParsedAddWidgetShape, Status> {
    let mut shape = FieldViolations::new();

    let scene_id = parse_uuid_field(&req.scene_id, "scene_id", &mut shape);
    let proto_pos = req.position.unwrap_or_default();
    let row = parse_grid_axis(proto_pos.row, "position.row", &mut shape);
    let col = parse_grid_axis(proto_pos.col, "position.col", &mut shape);
    let placement = parse_widget_size(req.size, "size", &mut shape);
    let config_req = req.config.or_else(|| {
        shape.push("config", "config is required");
        None
    });
    let widget_uid = config_req
        .as_ref()
        .and_then(|c| parse_uuid_field(&c.widget_uid, "config.widget_uid", &mut shape));

    if !shape.is_empty() {
        return Err(bad_request_status(shape));
    }

    match (scene_id, row, col, placement, config_req, widget_uid) {
        (
            Some(scene_id),
            Some(row),
            Some(col),
            Some(placement),
            Some(config_req),
            Some(widget_uid),
        ) => Ok(ParsedAddWidgetShape {
            scene_id,
            position: scene::WidgetPosition { row, col },
            placement,
            config_req,
            widget_uid,
        }),
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
    placement: scene::WidgetPlacement,
    params: Option<web::WidgetDataStruct>,
    /// Unlike `params`, `None` here keeps the stored bindings rather than clearing them.
    credential_bindings: Option<web::CredentialBindings>,
}

struct ValidatedWidgetUpdate {
    widget: scene::Widget,
    placement_changed: bool,
    params_changed: bool,
}

struct PendingWidgetUpdateCompletion {
    scene_id: scene::SceneId,
    widget_id: scene::WidgetId,
    replacement: Option<WidgetStopBatch>,
    registration: Option<PendingWidgetRegistration>,
    retry_params: bool,
    response_error: Option<Status>,
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
    let placement = parse_widget_size(req.size, "size", &mut shape);

    if !shape.is_empty() {
        return Err(bad_request_status(shape));
    }

    match (scene_id, widget_id, row, col, placement) {
        (Some(scene_id), Some(widget_id), Some(row), Some(col), Some(placement)) => {
            Ok(ParsedUpdateWidgetShape {
                scene_id,
                widget_id,
                proto_position,
                position: scene::WidgetPosition { row, col },
                placement,
                params: req.params,
                credential_bindings: req.credential_bindings,
            })
        }
        _ => Err(Status::internal(
            "BUG: shape parsing succeeded with missing required values",
        )),
    }
}

#[derive(Clone)]
pub(crate) struct SceneManagementService {
    widget_registry: Arc<WidgetRegistry>,
    config_handle: Arc<RwLock<ConfigHandle>>,
    /// Lock order wherever both are held: `config_handle` first, then this.
    secret_store: Arc<RwLock<SecretStoreHandle>>,
    coordinator: Arc<Coordinator>,
    capabilities: HardwareCapabilities,
    led_coordinator: LedCoordinatorHandle,
    /// Scene currently held open by a `preview_scene` stream. While set,
    /// that scene overrides normal compositor scene refreshes so edits made
    /// during a preview stay focused on it.
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
        secret_store: Arc<RwLock<SecretStoreHandle>>,
        coordinator: Arc<Coordinator>,
        capabilities: HardwareCapabilities,
        led_coordinator: LedCoordinatorHandle,
    ) -> Self {
        let preview_scene_id = coordinator.preview_scene_state();
        Self {
            widget_registry,
            config_handle,
            secret_store,
            coordinator,
            capabilities,
            led_coordinator,
            preview_scene_id,
        }
    }

    /// Refresh the compositor's scene state from current config. With no
    /// preview held this pushes the full cycling list; while a preview is
    /// active it instead pins that single scene so cycling and drag stay off.
    async fn refresh_compositor_scenes(&self) {
        let preview_id = *self.preview_scene_id.lock().await;
        let config = self.config_handle.read().await;
        if let Some(preview_id) = preview_id
            && let Some(scene) = config.scenes().get(&preview_id)
        {
            // A held preview pins exactly one scene so automatic cycling and
            // manual drag cannot move off the scene being edited. Other edits
            // are picked up by the full refresh when the preview ends.
            self.coordinator.pin_preview_scene(scene);
            return;
        }
        self.coordinator.refresh_scene_cycling(config.scenes());
    }

    /// Save config, returning a gRPC-friendly error on failure.
    async fn save_config(config: &mut ConfigHandle) -> Result<(), Status> {
        config
            .save()
            .await
            .map_err(|e| Status::internal(format!("failed to save config: {e}")))
    }

    fn validate_widget_update(
        &self,
        scene: &scene::Scene,
        widget_id: scene::WidgetId,
        parsed: ParsedUpdateWidgetShape,
        accounts: &IndexMap<AccountId, Account>,
    ) -> Result<ValidatedWidgetUpdate, Status> {
        let existing = scene
            .widgets
            .get(&widget_id)
            .ok_or_else(|| Status::not_found(format!("widget not found: {widget_id}")))?;
        let info = self
            .widget_registry
            .get(&existing.widget_type_id)
            .ok_or_else(|| Status::failed_precondition("widget manifest not installed"))?;
        let platform = PlatformDescriptor::from(&self.capabilities);
        let Some(descriptor) = platform.descriptor_for_placement(&parsed.placement) else {
            return Err(Status::failed_precondition(
                "size is not supported on this platform",
            ));
        };
        if !self
            .widget_registry
            .supports_viewport(&existing.widget_type_id, &descriptor)
        {
            return Err(Status::failed_precondition(format!(
                "widget {} does not support size viewport",
                existing.widget_type_id,
            )));
        }

        let typed_params = validate_widget_params(
            &info.manifest,
            &parsed.params.unwrap_or_default(),
            ValidateMode::Update,
        )
        .map_err(bad_request_status)?;
        let typed_bindings = match parsed.credential_bindings {
            None => existing.credential_bindings.clone(),
            Some(requested) => {
                validate_credential_bindings(&info.manifest, &requested.bindings, accounts)
                    .map_err(bad_request_status)?
            }
        };
        let mut widget = scene::Widget {
            id: widget_id,
            position: parsed.position,
            placement: parsed.placement,
            widget_type_id: existing.widget_type_id,
            viewport_shape: bmc_widget_manifest::ViewportShape::Rectangular,
            params: typed_params,
            credential_bindings: typed_bindings,
        };
        stamp_widget_viewport_shape_from_caps(&mut widget, &self.capabilities);
        validate_widget_placement(scene, &widget, Some(widget_id))?;

        Ok(ValidatedWidgetUpdate {
            placement_changed: existing.placement != widget.placement,
            params_changed: existing.params != widget.params,
            widget,
        })
    }

    async fn update_widget_transaction(
        self,
        parsed: ParsedUpdateWidgetShape,
    ) -> Result<Response<()>, Status> {
        let scene_id_key = scene::SceneId::from(parsed.scene_id);
        let widget_id_key = scene::WidgetId::from(parsed.widget_id);
        let completion = {
            let preview = self.preview_scene_id.lock().await;
            let mut config = self.config_handle.write().await;
            let accounts = self.secret_store.read().await;
            let scene = config.scenes_mut().get_mut(&scene_id_key).ok_or_else(|| {
                Status::not_found(format!("scene not found: {}", parsed.scene_id))
            })?;

            reject_update_widget_in_fullscreen(
                &scene.kind,
                parsed.proto_position,
                &parsed.placement,
            )?;
            if scene.kind == scene::SceneKind::Combined {
                reject_combined_when_no_slot_grid(&self.capabilities)?;
            }

            let ValidatedWidgetUpdate {
                widget,
                placement_changed,
                params_changed,
            } = self.validate_widget_update(scene, widget_id_key, parsed, accounts.accounts())?;
            let showing = scene.enabled || *preview == Some(scene_id_key);
            scene.widgets.insert(widget.id, widget.clone());
            Self::save_config(&mut config).await?;

            let replacement = if showing && placement_changed {
                Some(
                    self.coordinator
                        .enqueue_widget_replacement(WidgetInstanceKey::new(widget.id.as_uuid()))
                        .await,
                )
            } else {
                None
            };
            let registration = if placement_changed {
                self.coordinator.enqueue_configured_widget_registration(
                    &config,
                    accounts.accounts(),
                    scene_id_key,
                    widget_id_key,
                )
            } else {
                None
            };
            let mut response_error = None;
            let retry_params = if replacement.is_none() && params_changed {
                match self.coordinator.update_widget_params(
                    WidgetInstanceKey::new(widget.id.as_uuid()),
                    &widget.params,
                ) {
                    Ok(()) => true,
                    Err(error) => {
                        response_error = Some(live_params_status(&error));
                        false
                    }
                }
            } else {
                false
            };
            PendingWidgetUpdateCompletion {
                scene_id: scene_id_key,
                widget_id: widget_id_key,
                replacement,
                registration,
                retry_params,
                response_error,
            }
        };

        self.finish_widget_update(completion).await
    }

    async fn finish_widget_update(
        &self,
        completion: PendingWidgetUpdateCompletion,
    ) -> Result<Response<()>, Status> {
        let retry = completion.retry_params;
        if let Some(cutoff) = completion.replacement {
            cutoff.wait().await;
        }
        if let Some(registration) = completion.registration {
            self.coordinator
                .finish_widget_registration(&self.config_handle, completion.scene_id, registration)
                .await;
        }
        if retry {
            self.coordinator
                .retry_pending_widget(&completion.widget_id.to_string())
                .await;
        }
        self.refresh_compositor_scenes().await;

        completion
            .response_error
            .map_or_else(|| Ok(Response::new(())), Err)
    }
}

fn params_to_widget_data_struct(
    params: &BTreeMap<bmc_widget_manifest::ParamKey, bmc_widget_manifest::ParamValue>,
) -> web::WidgetDataStruct {
    web::WidgetDataStruct {
        fields: params
            .iter()
            .map(|(k, v)| (k.as_str().to_owned(), param_value_to_wire(v)))
            .collect(),
    }
}

fn param_value_to_wire(v: &bmc_widget_manifest::ParamValue) -> web::WidgetDataValue {
    use bmc_widget_manifest::ParamValue as PV;
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

pub(crate) fn param_definition_to_proto(
    key: &str,
    def: &ParamDefinition,
) -> web::ManifestParamDefinition {
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

fn string_format_to_proto(f: bmc_widget_manifest::StringFormat) -> web::StringFormat {
    use bmc_widget_manifest::StringFormat as F;
    match f {
        F::Date => web::StringFormat::Date,
        F::Time => web::StringFormat::Time,
        F::Email => web::StringFormat::Email,
        F::Uri => web::StringFormat::Uri,
        F::Password => web::StringFormat::Password,
    }
}

pub(crate) fn category_to_proto(c: bmc_widget_manifest::WidgetCategory) -> web::WidgetCategory {
    use bmc_widget_manifest::WidgetCategory as C;
    match c {
        C::Mining => web::WidgetCategory::Mining,
        C::Clock => web::WidgetCategory::Clock,
        C::Weather => web::WidgetCategory::Weather,
        C::Calendar => web::WidgetCategory::Calendar,
        C::Space => web::WidgetCategory::Space,
        C::Knowledge => web::WidgetCategory::Knowledge,
        C::Utility => web::WidgetCategory::Utility,
        C::Media => web::WidgetCategory::Media,
        C::Finance => web::WidgetCategory::Finance,
        C::Misc => web::WidgetCategory::Misc,
    }
}

fn widget_info_to_proto(
    info: &crate::widget::WidgetInfo,
    platform: &PlatformDescriptor,
) -> web::WidgetManifest {
    let manifest = &info.manifest;
    web::WidgetManifest {
        uid: manifest.uid.to_string(),
        name: manifest.name.clone(),
        subname: manifest.subname.clone(),
        description: manifest.description.clone(),
        config_help: manifest.config_help.clone(),
        version: manifest.version.to_string(),
        supported_sizes: supported_sizes_for_constraints(platform, &manifest.supported_viewports)
            .into_iter()
            .map(Into::into)
            .collect(),
        params: manifest
            .params
            .iter()
            .map(|(key, param)| param_definition_to_proto(key.as_str(), param))
            .collect(),
        credentials: manifest
            .credentials
            .iter()
            .map(|(key, slot)| web::CredentialSlotDefinition {
                key: key.as_str().to_owned(),
                type_id: slot.type_id.clone(),
                label: slot.label.clone(),
                description: slot.description.clone(),
                required: slot.required,
            })
            .collect(),
        // BMC icon endpoint when an icon exists; absent → FE fallback glyph.
        icon_url: info
            .icon_path
            .as_ref()
            .map(|_| format!("/widgets/{}/icon", manifest.uid)),
        category: i32::from(category_to_proto(manifest.category)),
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
            "widget at ({},{}) size {:?} is out of grid bounds",
            widget.position.row, widget.position.col, widget.placement,
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
    new_placement: &scene::WidgetPlacement,
) -> Result<(), Status> {
    if *kind != scene::SceneKind::Fullscreen {
        return Ok(());
    }
    if new_position.row != 0 || new_position.col != 0 {
        return Err(Status::failed_precondition(
            "cannot move widget in fullscreen scene",
        ));
    }
    if *new_placement != scene::WidgetPlacement::Fullscreen {
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
/// Validate a `WidgetDataStruct` against the manifest and project it onto
/// a typed `BTreeMap<ParamKey, ParamValue>`. On `Add`, manifest keys missing
/// from `params` are seeded with the manifest's default (or `Null` for
/// optional params without a default). On `Update`, missing keys are
/// reported as violations. Unknown override keys are also reported.
pub(crate) fn validate_widget_params(
    manifest: &bmc_widget_manifest::Manifest,
    params: &web::WidgetDataStruct,
    mode: ValidateMode,
) -> Result<BTreeMap<bmc_widget_manifest::ParamKey, bmc_widget_manifest::ParamValue>, FieldViolations>
{
    use web::widget_data_value::Kind as VK;
    let mut violations = FieldViolations::new();
    let mut typed = BTreeMap::new();

    for (key, def) in &manifest.params {
        let path = format!("params[{:?}]", key.as_str());
        let Some(wdv) = params.fields.get(key.as_str()) else {
            if matches!(mode, ValidateMode::Update) {
                violations.push(path, "Value is required");
            } else {
                typed.insert(
                    key.clone(),
                    bmc_widget_manifest::ParamValue::from_param_kind_default(&def.kind),
                );
            }
            continue;
        };

        let Some(kind) = wdv.kind.as_ref() else {
            violations.push(path, "WidgetDataValue.kind unset");
            continue;
        };

        if matches!(kind, VK::NullValue(())) {
            if def.is_optional {
                typed.insert(key.clone(), bmc_widget_manifest::ParamValue::Null);
            } else {
                violations.push(path, "Value is required");
            }
            continue;
        }

        if let Some(value) =
            validate_and_project_param_value(&path, &def.kind, kind, &mut violations)
        {
            typed.insert(key.clone(), value);
        }
    }

    for key in params.fields.keys() {
        if !manifest.params.contains_key(key.as_str()) {
            // Debug-format the key so a client-supplied key with quotes,
            // newlines, etc. cannot break the FieldViolation field path.
            violations.push(format!("params[{key:?}]"), "Unknown param");
        }
    }

    if violations.is_empty() {
        Ok(typed)
    } else {
        Err(violations)
    }
}

/// Validate a binding set against the manifest's slots and the stored accounts,
/// projecting it onto a typed map. A required slot left unbound is *not* a violation:
/// an operator may save a widget before creating the account it needs.
///
/// Callers read `accounts` under the config write lock that also inserts the result:
/// a removal slipping between check and insert would persist a binding
/// with no account behind it, the state the delete cascade exists to prevent.
pub(crate) fn validate_credential_bindings(
    manifest: &bmc_widget_manifest::Manifest,
    bindings: &HashMap<String, String>,
    accounts: &IndexMap<AccountId, Account>,
) -> Result<BTreeMap<CredentialKey, AccountId>, FieldViolations> {
    let mut violations = FieldViolations::new();
    let mut typed = BTreeMap::new();

    for (key, slot) in &manifest.credentials {
        // The picker's "— None —" sends an empty id rather than dropping the key.
        let Some(raw) = bindings.get(key.as_str()).filter(|id| !id.is_empty()) else {
            continue;
        };
        let path = format!("credential_bindings[{:?}]", key.as_str());
        match AccountId::from_str(raw)
            .ok()
            .and_then(|id| accounts.get_key_value(&id))
        {
            None => violations.push(path, "Account not found"),
            Some((_, account)) if account.type_id != slot.type_id => {
                violations.push(path, "Account is of a different credential type");
            }
            Some((id, _)) => {
                typed.insert(key.clone(), id.clone());
            }
        }
    }

    for key in bindings.keys() {
        if !manifest.credentials.contains_key(key.as_str()) {
            violations.push(
                format!("credential_bindings[{key:?}]"),
                "Unknown credential slot",
            );
        }
    }

    if violations.is_empty() {
        Ok(typed)
    } else {
        Err(violations)
    }
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

fn validate_and_project_param_value(
    path: &str,
    param_kind: &ParamKind,
    kind: &web::widget_data_value::Kind,
    violations: &mut FieldViolations,
) -> Option<bmc_widget_manifest::ParamValue> {
    use bmc_widget_manifest::ParamValue as PV;
    use web::widget_data_value::Kind as VK;
    match (param_kind, kind) {
        (ParamKind::String { enum_values, .. }, VK::StringValue(s)) => {
            if !enum_values.is_empty() && !enum_values.iter().any(|o| &o.value == s) {
                violations.push(path.to_owned(), "Must be one of the listed options");
                return None;
            }
            Some(PV::String(s.clone()))
        }
        (ParamKind::Timezone { .. }, VK::StringValue(s)) => {
            if !Timezone::list().iter().any(|tz| tz.iana() == s) {
                violations.push(path.to_owned(), "Must be a valid timezone");
                return None;
            }
            Some(PV::String(s.clone()))
        }
        (ParamKind::Boolean { .. }, VK::BooleanValue(b)) => Some(PV::Boolean(*b)),
        (
            ParamKind::Integer {
                min,
                max,
                enum_values,
                ..
            },
            VK::IntegerValue(i),
        ) => {
            let mut ok = true;
            if let Some(lo) = min
                && i < lo
            {
                violations.push(path.to_owned(), format!("Must be at least {lo}"));
                ok = false;
            }
            if let Some(hi) = max
                && i > hi
            {
                violations.push(path.to_owned(), format!("Must be at most {hi}"));
                ok = false;
            }
            if !enum_values.is_empty() && !enum_values.iter().any(|o| o.value == *i) {
                violations.push(path.to_owned(), "Must be one of the listed options");
                ok = false;
            }
            if ok { Some(PV::Integer(*i)) } else { None }
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
                return None;
            }
            let mut ok = true;
            if let Some(lo) = min
                && d < lo
            {
                violations.push(path.to_owned(), format!("Must be at least {lo}"));
                ok = false;
            }
            if let Some(hi) = max
                && d > hi
            {
                violations.push(path.to_owned(), format!("Must be at most {hi}"));
                ok = false;
            }
            if !enum_values.is_empty()
                && !enum_values.iter().any(|o| {
                    bmc_widget_manifest::f64_canonical_bits(o.value)
                        == bmc_widget_manifest::f64_canonical_bits(*d)
                })
            {
                violations.push(path.to_owned(), "Must be one of the listed options");
                ok = false;
            }
            if ok { Some(PV::Double(*d)) } else { None }
        }
        (other_kind, _) => {
            violations.push(path.to_owned(), type_mismatch_message(other_kind));
            None
        }
    }
}

fn scene_widget_to_proto(
    widget: &scene::Widget,
    accounts: &IndexMap<AccountId, Account>,
) -> web::Widget {
    web::Widget {
        id: widget.id.to_string(),
        position: Some(web::WidgetPosition {
            row: u32::from(widget.position.row),
            col: u32::from(widget.position.col),
        }),
        size: web::WidgetSize::try_from(&widget.placement)
            .unwrap_or(web::WidgetSize::Unspecified)
            .into(),
        config: Some(web::WidgetConfig {
            widget_uid: widget.widget_type_id.to_string(),
            params: Some(params_to_widget_data_struct(&widget.params)),
            credential_bindings: Some(web::CredentialBindings {
                bindings: credential::effective_bindings(&widget.credential_bindings, accounts)
                    .map(|(key, account)| (key.as_str().to_owned(), account.id.to_string()))
                    .collect(),
            }),
        }),
    }
}

fn scene_to_proto(scene: &scene::Scene, accounts: &IndexMap<AccountId, Account>) -> web::Scene {
    let widgets: Vec<web::Widget> = scene
        .widgets
        .values()
        .map(|widget| scene_widget_to_proto(widget, accounts))
        .collect();

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
        Ok(web::SceneCyclingTransition::None) => Ok(SceneCyclingTransition::None),
        Ok(web::SceneCyclingTransition::Unspecified) => Err(Status::invalid_argument(
            "scene_cycling.transition must be Slide, Fade, or None (got Unspecified)",
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
        let platform = PlatformDescriptor::from(&self.capabilities);
        let available = self.widget_registry.list();
        let widgets = available
            .iter()
            .map(|info| widget_info_to_proto(info, &platform))
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

        let platform = PlatformDescriptor::from(&self.capabilities);
        Ok(Response::new(widget_info_to_proto(&info, &platform)))
    }

    // ── Scene read RPCs (from config) ──────────────────────────────────

    async fn get_scenes(
        &self,
        _request: Request<()>,
    ) -> Result<Response<web::GetScenesResponse>, Status> {
        let config = self.config_handle.read().await;
        let store = self.secret_store.read().await;
        let scenes = config
            .scenes()
            .values()
            .map(|scene| scene_to_proto(scene, store.accounts()))
            .collect();
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
            .scenes()
            .get(&scene::SceneId::from(id))
            .ok_or_else(|| Status::not_found(format!("scene not found: {id}")))?;

        let store = self.secret_store.read().await;
        Ok(Response::new(web::SceneResponse {
            scene: Some(scene_to_proto(scene, store.accounts())),
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

        let platform = PlatformDescriptor::from(&self.capabilities);
        if !self
            .widget_registry
            .supports_viewport(&widget_uid, &platform.fullscreen)
        {
            return Err(Status::failed_precondition(format!(
                "widget {widget_uid} does not support the active fullscreen viewport",
            )));
        }

        let params = config.params.unwrap_or_default();
        let typed_params = validate_widget_params(manifest, &params, ValidateMode::Add)
            .map_err(bad_request_status)?;
        let requested_bindings = config.credential_bindings.unwrap_or_default().bindings;

        let mut scene = scene::Scene::fullscreen(widget_uid, typed_params);
        for widget in scene.widgets.values_mut() {
            stamp_widget_viewport_shape_from_caps(widget, &self.capabilities);
        }
        let scene_id = scene.id.to_string();

        let scene_key = scene.id;

        let registrations;
        {
            let mut config = self.config_handle.write().await;
            let accounts = self.secret_store.read().await;
            let typed_bindings =
                validate_credential_bindings(manifest, &requested_bindings, accounts.accounts())
                    .map_err(bad_request_status)?;
            for widget in scene.widgets.values_mut() {
                widget.credential_bindings = typed_bindings.clone();
            }
            config.scenes_mut().insert(scene.id, scene);
            Self::save_config(&mut config).await?;
            registrations = self.coordinator.enqueue_configured_scene_registrations(
                &config,
                accounts.accounts(),
                scene_key,
            );
        }

        for registration in registrations {
            self.coordinator
                .finish_widget_registration(&self.config_handle, scene_key, registration)
                .await;
        }
        self.refresh_compositor_scenes().await;

        Ok(Response::new(scene_id))
    }

    async fn add_combined_scene(&self, _request: Request<()>) -> Result<Response<String>, Status> {
        reject_combined_when_no_slot_grid(&self.capabilities)?;

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
            config.scenes_mut().insert(scene.id, scene);
            Self::save_config(&mut config).await?;
        }

        self.refresh_compositor_scenes().await;

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
        let (stop, start) = {
            let preview = self.preview_scene_id.lock().await;
            let previewing = *preview == Some(scene_id_key);
            let mut config = self.config_handle.write().await;
            let scene = config
                .scenes_mut()
                .get_mut(&scene_id_key)
                .ok_or_else(|| Status::not_found(format!("scene not found: {id}")))?;

            let cycle_duration = req
                .cycle_duration_sec
                .map(|s| Duration::from_secs(u64::from(s)));
            if cycle_duration.is_some_and(|duration| duration < scene::Scene::MIN_CYCLE_DURATION) {
                return Err(Status::invalid_argument(format!(
                    "cycle_duration is shorter than the minimum {:?}",
                    scene::Scene::MIN_CYCLE_DURATION,
                )));
            }

            let was_enabled = scene.enabled;
            let was_shown = was_enabled || previewing;
            scene.enabled = req.enabled;
            scene.cycle_duration = cycle_duration;
            let is_shown = scene.enabled || previewing;
            let became_enabled = !was_enabled && scene.enabled;
            let scene_to_stop = (was_shown && !is_shown).then(|| scene.clone());

            Self::save_config(&mut config).await?;
            if let Some(scene) = scene_to_stop {
                (
                    Some(self.coordinator.enqueue_scene_stop(&scene).await),
                    false,
                )
            } else {
                (None, became_enabled)
            }
        };

        if let Some(stop) = stop {
            stop.wait().await;
        } else if start {
            self.coordinator
                .spawn_configured_scene_widgets(&self.config_handle, scene_id_key)
                .await;
        }

        self.refresh_compositor_scenes().await;

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
            .scenes()
            .get_index_of(&scene_id)
            .ok_or_else(|| Status::not_found(format!("scene not found: {id}")))?;

        let target_idx = req.index as usize;
        if target_idx >= config.scenes().len() {
            return Err(Status::invalid_argument(format!(
                "target index {target_idx} out of bounds (scene count: {})",
                config.scenes().len()
            )));
        }
        config.scenes_mut().move_index(current_idx, target_idx);

        Self::save_config(&mut config).await?;
        drop(config);

        self.refresh_compositor_scenes().await;

        Ok(Response::new(()))
    }

    async fn clone_scene(&self, request: Request<String>) -> Result<Response<String>, Status> {
        let id_str = request.into_inner();
        let id =
            Uuid::parse_str(&id_str).map_err(|_| Status::invalid_argument("invalid scene ID"))?;

        let mut config = self.config_handle.write().await;
        let (source_idx, mut cloned) = config
            .scenes()
            .get_full(&scene::SceneId::from(id))
            .map(|(idx, _, scene)| (idx, scene.clone()))
            .ok_or_else(|| Status::not_found(format!("scene not found: {id}")))?;

        cloned.id = scene::SceneId::generate();
        // Give each widget a new ID
        let old_widgets = std::mem::take(&mut cloned.widgets);
        for (_, widget) in old_widgets {
            let new_widget = widget.clone_with_new_id();
            cloned.widgets.insert(new_widget.id, new_widget);
        }

        let cloned_id = cloned.id.to_string();
        let cloned_key = cloned.id;
        // Insert right after the source scene
        let scenes = config.scenes_mut();
        scenes.insert(cloned.id, cloned);
        let last_idx = scenes.len() - 1;
        scenes.move_index(last_idx, source_idx + 1);

        Self::save_config(&mut config).await?;
        let registrations = {
            let accounts = self.secret_store.read().await;
            self.coordinator.enqueue_configured_scene_registrations(
                &config,
                accounts.accounts(),
                cloned_key,
            )
        };
        drop(config);

        for registration in registrations {
            self.coordinator
                .finish_widget_registration(&self.config_handle, cloned_key, registration)
                .await;
        }
        self.refresh_compositor_scenes().await;

        Ok(Response::new(cloned_id))
    }

    async fn remove_scene(&self, request: Request<String>) -> Result<Response<()>, Status> {
        let id_str = request.into_inner();
        let id =
            Uuid::parse_str(&id_str).map_err(|_| Status::invalid_argument("invalid scene ID"))?;
        let scene_id_key = scene::SceneId::from(id);

        let preview = self.preview_scene_id.lock().await;
        if preview.as_ref() == Some(&scene_id_key) {
            return Err(Status::failed_precondition(
                "scene is currently being previewed",
            ));
        }

        let stop = {
            let mut config = self.config_handle.write().await;
            let removed_scene = config
                .scenes_mut()
                .shift_remove(&scene_id_key)
                .ok_or_else(|| Status::not_found(format!("scene not found: {id}")))?;
            Self::save_config(&mut config).await?;
            self.coordinator.enqueue_scene_delete(&removed_scene).await
        };
        drop(preview);

        stop.wait().await;
        self.refresh_compositor_scenes().await;

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

        struct PreviewGuard {
            coordinator: Arc<Coordinator>,
            config_handle: Arc<RwLock<ConfigHandle>>,
            preview_scene_id: Arc<Mutex<Option<scene::SceneId>>>,
            led_coordinator: LedCoordinatorHandle,
        }
        impl Drop for PreviewGuard {
            fn drop(&mut self) {
                self.led_coordinator.publish(Layer::Preview, None);
                let coordinator = Arc::clone(&self.coordinator);
                let config_handle = Arc::clone(&self.config_handle);
                let preview_scene_id = Arc::clone(&self.preview_scene_id);
                tokio::spawn(async move {
                    let mut preview = preview_scene_id.lock().await;
                    let stop = {
                        let config = config_handle.read().await;
                        match preview.as_ref().and_then(|id| config.scenes().get(id)) {
                            Some(scene) if !scene.enabled => {
                                Some(coordinator.enqueue_scene_stop(scene).await)
                            }
                            _ => None,
                        }
                    };
                    if let Some(stop) = stop {
                        stop.wait().await;
                    }
                    let config = config_handle.read().await;
                    coordinator.refresh_scene_cycling(config.scenes());
                    // Release after restoration so a successor cannot pin before it.
                    preview.take();
                });
            }
        }

        let scene_was_disabled = {
            let mut preview = self.preview_scene_id.lock().await;
            if preview.is_some() {
                return Err(Status::resource_exhausted("scene preview already active"));
            }

            let config = self.config_handle.read().await;
            let scene = config
                .scenes()
                .get(&scene_id)
                .ok_or_else(|| Status::not_found(format!("scene not found: {scene_id}")))?;

            *preview = Some(scene_id);

            !scene.enabled
        };
        let guard = PreviewGuard {
            coordinator: Arc::clone(&self.coordinator),
            config_handle: Arc::clone(&self.config_handle),
            preview_scene_id: Arc::clone(&self.preview_scene_id),
            led_coordinator: self.led_coordinator.clone(),
        };
        if scene_was_disabled {
            self.coordinator
                .spawn_configured_scene_widgets(&self.config_handle, scene_id)
                .await;
        }
        self.refresh_compositor_scenes().await;
        self.led_coordinator.publish(
            Layer::Preview,
            Some(bmc_led::data::LedScene {
                effect: bmc_led::data::LedEffect::Solid(bmc_led::config::RGB_WHITE),
                period: None,
                duration: None,
            }),
        );

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

        let stop = {
            let mut config = self.config_handle.write().await;
            let scene = config
                .scenes_mut()
                .get_mut(&scene::SceneId::from(scene_id))
                .ok_or_else(|| Status::not_found(format!("scene not found: {scene_id}")))?;

            reject_remove_widget_in_fullscreen(&scene.kind)?;
            if scene.kind == scene::SceneKind::Combined {
                reject_combined_when_no_slot_grid(&self.capabilities)?;
            }

            scene
                .widgets
                .shift_remove(&scene::WidgetId::from(widget_id))
                .ok_or_else(|| Status::not_found(format!("widget not found: {widget_id}")))?;

            Self::save_config(&mut config).await?;
            self.coordinator
                .enqueue_widget_delete(WidgetInstanceKey::new(widget_id))
                .await
        };

        stop.wait().await;
        self.refresh_compositor_scenes().await;

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
                    SceneCyclingTransition::None => web::SceneCyclingTransition::None.into(),
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

        let default_duration =
            Duration::from_secs(u64::from(cycling.automatic_cycling_default_duration_sec));
        if default_duration < scene::Scene::MIN_CYCLE_DURATION {
            return Err(Status::invalid_argument(format!(
                "automatic_cycling_default_duration is shorter than the minimum {:?}",
                scene::Scene::MIN_CYCLE_DURATION,
            )));
        }

        let config_to_apply = SceneCycling {
            automatic_cycling_enabled: cycling.automatic_cycling_enabled,
            automatic_cycling_default_duration: default_duration,
            transition: parse_scene_cycling_transition(cycling.transition)?,
        };

        let mut config = self.config_handle.write().await;
        config.set_scene_cycling(config_to_apply.clone());
        Self::save_config(&mut config).await?;
        drop(config);

        if let Err(err) = self
            .coordinator
            .compositor()
            .set_scene_cycling_config(config_to_apply)
        {
            tracing::warn!(error = %err, "failed to apply scene cycling config");
        }

        Ok(Response::new(()))
    }

    async fn add_widget(
        &self,
        request: Request<web::AddWidgetRequest>,
    ) -> Result<Response<String>, Status> {
        let ParsedAddWidgetShape {
            scene_id,
            position,
            placement,
            config_req,
            widget_uid,
        } = parse_add_widget_shape(request.into_inner())?;

        let info = self
            .widget_registry
            .get(&widget_uid)
            .ok_or_else(|| Status::failed_precondition("widget manifest not installed"))?;
        let manifest = &info.manifest;

        let platform = PlatformDescriptor::from(&self.capabilities);
        let Some(descriptor) = platform.descriptor_for_placement(&placement) else {
            return Err(Status::failed_precondition(
                "size is not supported on this platform",
            ));
        };
        if !self
            .widget_registry
            .supports_viewport(&widget_uid, &descriptor)
        {
            return Err(Status::failed_precondition(format!(
                "widget {widget_uid} does not support size viewport",
            )));
        }

        let params = config_req.params.unwrap_or_default();
        let typed_params = validate_widget_params(manifest, &params, ValidateMode::Add)
            .map_err(bad_request_status)?;

        let mut widget = scene::Widget::new(widget_uid, typed_params, position, placement);
        stamp_widget_viewport_shape_from_caps(&mut widget, &self.capabilities);
        let widget_id = widget.id.to_string();
        let requested_bindings = config_req.credential_bindings.unwrap_or_default().bindings;

        let scene_id_key = scene::SceneId::from(scene_id);

        let registration = {
            let mut config = self.config_handle.write().await;
            let accounts = self.secret_store.read().await;
            widget.credential_bindings =
                validate_credential_bindings(manifest, &requested_bindings, accounts.accounts())
                    .map_err(bad_request_status)?;
            let scene = config
                .scenes_mut()
                .get_mut(&scene_id_key)
                .ok_or_else(|| Status::not_found(format!("scene not found: {scene_id}")))?;

            if scene.kind == scene::SceneKind::Combined {
                reject_combined_when_no_slot_grid(&self.capabilities)?;
            }

            validate_widget_placement(scene, &widget, None)?;

            scene.widgets.insert(widget.id, widget.clone());
            Self::save_config(&mut config).await?;

            self.coordinator.enqueue_configured_widget_registration(
                &config,
                accounts.accounts(),
                scene_id_key,
                widget.id,
            )
        };

        if let Some(registration) = registration {
            self.coordinator
                .finish_widget_registration(&self.config_handle, scene_id_key, registration)
                .await;
        }
        self.refresh_compositor_scenes().await;

        Ok(Response::new(widget_id))
    }

    async fn update_widget(
        &self,
        request: Request<web::UpdateWidgetRequest>,
    ) -> Result<Response<()>, Status> {
        let parsed = parse_update_widget_shape(request.into_inner())?;
        tokio::spawn(self.clone().update_widget_transaction(parsed))
            .await
            .map_err(|error| Status::internal(format!("widget update task failed: {error}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::compositor::testing::RecordingCompositor;
    use std::os::unix::fs::PermissionsExt;

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
            &scene::WidgetPlacement::SlotSpan(scene::SlotSpan {
                columns: 1,
                rows: 1,
            }),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn reject_update_widget_in_fullscreen_allows_full_size_at_origin() {
        let pos = web::WidgetPosition { row: 0, col: 0 };
        let result = reject_update_widget_in_fullscreen(
            &scene::SceneKind::Fullscreen,
            pos,
            &scene::WidgetPlacement::Fullscreen,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn reject_update_widget_in_fullscreen_rejects_position_change() {
        let pos = web::WidgetPosition { row: 1, col: 0 };
        let err = reject_update_widget_in_fullscreen(
            &scene::SceneKind::Fullscreen,
            pos,
            &scene::WidgetPlacement::Fullscreen,
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
            &scene::WidgetPlacement::SlotSpan(scene::SlotSpan {
                columns: 1,
                rows: 1,
            }),
        )
        .expect_err("BUG: must reject resized fullscreen widget");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("resize"));
    }

    #[test]
    fn build_widget_params_seeds_required_with_default() {
        use bmc_widget_manifest::ParamValue as PV;
        let manifest = single_param_manifest(
            "name",
            ParamKind::String {
                format: None,
                enum_values: vec![],
                default_value: Some("hello".into()),
            },
            false,
        );
        let resolved = validate_widget_params(
            &manifest,
            &web::WidgetDataStruct::default(),
            ValidateMode::Add,
        )
        .expect("BUG: defaults-only must validate");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved.get("name"), Some(&PV::String("hello".into())));
    }

    #[test]
    fn build_widget_params_seeds_optional_no_default_with_null() {
        use bmc_widget_manifest::ParamValue as PV;
        let manifest = single_param_manifest(
            "name",
            ParamKind::String {
                format: None,
                enum_values: vec![],
                default_value: None,
            },
            true,
        );
        let resolved = validate_widget_params(
            &manifest,
            &web::WidgetDataStruct::default(),
            ValidateMode::Add,
        )
        .expect("BUG: defaults-only must validate");
        assert_eq!(resolved.get("name"), Some(&PV::Null));
    }

    #[test]
    fn build_widget_params_override_wins() {
        use bmc_widget_manifest::ParamValue as PV;
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
        let resolved = validate_widget_params(&manifest, &overrides, ValidateMode::Add)
            .expect("BUG: override must validate");
        assert_eq!(resolved.get("name"), Some(&PV::String("world".into())));
    }

    #[test]
    fn build_widget_params_seeds_each_kind_with_default() {
        use bmc_widget_manifest::ParamValue as PV;
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
        let resolved = validate_widget_params(
            &manifest,
            &web::WidgetDataStruct::default(),
            ValidateMode::Add,
        )
        .expect("BUG: defaults-only must validate");
        assert_eq!(resolved.len(), 5);
        assert_eq!(resolved.get("s"), Some(&PV::String("x".into())));
        assert_eq!(resolved.get("i"), Some(&PV::Integer(7)));
        assert_eq!(resolved.get("d"), Some(&PV::Double(2.5)));
        assert_eq!(resolved.get("b"), Some(&PV::Boolean(true)));
        assert_eq!(resolved.get("t"), Some(&PV::String("UTC".into())));
    }

    #[test]
    fn params_to_widget_data_struct_round_trips_each_arm() {
        use bmc_widget_manifest::{ParamKey, ParamValue as PV};
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
        kind: bmc_widget_manifest::ParamKind,
        is_optional: bool,
    ) -> bmc_widget_manifest::Manifest {
        let param = bmc_widget_manifest::ParamDefinition {
            name: "Test".into(),
            description: None,
            is_optional,
            kind,
        };
        let pk: bmc_widget_manifest::ParamKey =
            serde_json::from_str(&format!("\"{key}\"")).expect("BUG: valid key");
        let mut params = indexmap::IndexMap::new();
        params.insert(pk, param);
        bmc_widget_manifest::Manifest {
            uid: uuid::Uuid::new_v4(),
            version: semver::Version::new(1, 0, 0),
            name: "T".into(),
            subname: None,
            description: "T".into(),
            config_help: None,
            author: None,
            binary: std::path::PathBuf::from("bin/test"),
            icon: None,
            category: bmc_widget_manifest::WidgetCategory::Misc,
            settings: vec![],
            supported_viewports: vec![bmc_widget_manifest::WidgetViewportConstraint {
                viewport_shape: bmc_widget_manifest::ViewportShape::Rectangular,
                min_width: Some(317),
                max_width: Some(317),
                min_height: Some(238),
                max_height: Some(238),
                min_dpi: Some(1),
                max_dpi: Some(1),
            }],
            params,
            credentials: indexmap::IndexMap::new(),
        }
    }

    fn manifest_with_params(
        entries: &[(&str, bmc_widget_manifest::ParamKind, bool)],
    ) -> bmc_widget_manifest::Manifest {
        let mut params = indexmap::IndexMap::new();
        for (key, kind, is_optional) in entries {
            let pk: bmc_widget_manifest::ParamKey =
                serde_json::from_str(&format!("\"{key}\"")).expect("BUG: valid key");
            let param = bmc_widget_manifest::ParamDefinition {
                name: "Test".into(),
                description: None,
                is_optional: *is_optional,
                kind: kind.clone(),
            };
            params.insert(pk, param);
        }
        bmc_widget_manifest::Manifest {
            uid: uuid::Uuid::new_v4(),
            version: semver::Version::new(1, 0, 0),
            name: "T".into(),
            subname: None,
            description: "T".into(),
            config_help: None,
            author: None,
            binary: std::path::PathBuf::from("bin/test"),
            icon: None,
            category: bmc_widget_manifest::WidgetCategory::Misc,
            settings: vec![],
            supported_viewports: vec![bmc_widget_manifest::WidgetViewportConstraint {
                viewport_shape: bmc_widget_manifest::ViewportShape::Rectangular,
                min_width: Some(317),
                max_width: Some(317),
                min_height: Some(238),
                max_height: Some(238),
                min_dpi: Some(1),
                max_dpi: Some(1),
            }],
            params,
            credentials: indexmap::IndexMap::new(),
        }
    }

    fn fields_one(key: &str, value: web::WidgetDataValue) -> web::WidgetDataStruct {
        web::WidgetDataStruct {
            fields: [(key.to_owned(), value)].into_iter().collect(),
        }
    }

    /// `keep()` because the handle outlives this call: a `TempDir` dropped here
    /// would leave it pointing into a deleted directory.
    async fn empty_secret_store() -> Arc<RwLock<SecretStoreHandle>> {
        let dir = tempfile::tempdir().expect("BUG: tempdir").keep();
        let store = SecretStoreHandle::init(&dir.join("config.json")).await;
        Arc::new(RwLock::new(store))
    }

    fn manifest_with_credentials(entries: &[(&str, &str, bool)]) -> bmc_widget_manifest::Manifest {
        let mut manifest = manifest_with_params(&[]);
        for (key, type_id, required) in entries {
            let key: CredentialKey =
                serde_json::from_str(&format!("\"{key}\"")).expect("BUG: valid key");
            let slot = bmc_widget_manifest::CredentialSlot {
                type_id: (*type_id).to_owned(),
                label: "Test".into(),
                description: None,
                required: *required,
            };
            manifest.credentials.insert(key, slot);
        }
        manifest
    }

    fn account_of_type(id: &str, type_id: &str) -> (AccountId, Account) {
        let id = AccountId::from_str(id).expect("BUG: non-empty id");
        let account = Account {
            id: id.clone(),
            type_id: type_id.to_owned(),
            name: "Test".into(),
            field_values: indexmap::IndexMap::new(),
            allow_hosts: Vec::new(),
            created_at: chrono::Utc::now(),
        };
        (id, account)
    }

    /// Build a coordinator whose registry holds exactly one widget, declaring `slots`.
    async fn coordinator_declaring(
        widget_type_id: Uuid,
        slots: &[(&str, &str, bool)],
        secret_store: Arc<RwLock<SecretStoreHandle>>,
    ) -> (Coordinator, Arc<RecordingCompositor>) {
        let mut manifest = manifest_with_credentials(slots);
        manifest.uid = widget_type_id;
        let registry = Arc::new(WidgetRegistry::new(vec![
            crate::widget::WidgetInfo::for_test(
                manifest,
                std::path::PathBuf::from("/test/widgets/test-widget"),
                std::path::PathBuf::from("/test/widgets/test-widget/bin/widget"),
                None,
            ),
        ]));
        let compositor = Arc::new(RecordingCompositor::default());
        let compositor_trait: Arc<dyn crate::compositor::Compositor> = compositor.clone();
        let widget_manager = crate::widget::WidgetManager::init(Vec::new(), false).await;
        (
            Coordinator::new(
                widget_manager,
                compositor_trait,
                Some("test-display".to_owned()),
                registry,
                bmc100_caps(None),
                secret_store,
            ),
            compositor,
        )
    }

    async fn store_holding(id: &str, type_id: &str) -> (Arc<RwLock<SecretStoreHandle>>, AccountId) {
        let store = empty_secret_store().await;
        let (account_id, account) = account_of_type(id, type_id);
        store
            .write()
            .await
            .accounts_mut()
            .insert(account_id.clone(), account);
        (store, account_id)
    }

    fn widget_bound_to(widget_type_id: Uuid, slot: &str, account_id: &AccountId) -> scene::Widget {
        let mut widget = scene::Scene::fullscreen(widget_type_id, BTreeMap::new())
            .widgets
            .into_values()
            .next()
            .expect("BUG: a fullscreen scene holds one widget");
        let key: CredentialKey =
            serde_json::from_str(&format!("\"{slot}\"")).expect("BUG: valid key");
        widget.credential_bindings.insert(key, account_id.clone());
        widget
    }

    /// The pair matters: without the positive case, a resolver that always yielded
    /// nothing would satisfy the negative one.
    #[tokio::test]
    async fn resolution_honours_a_slot_the_installed_manifest_declares() {
        let widget_type_id = Uuid::new_v4();
        let (store, account_id) =
            store_holding("a-1", credential::BuiltinType::BraiinsPool.id()).await;
        let (coordinator, _) = coordinator_declaring(
            widget_type_id,
            &[("pool", credential::BuiltinType::BraiinsPool.id(), true)],
            store,
        )
        .await;

        let resolved = coordinator
            .resolve_credentials(&widget_bound_to(widget_type_id, "pool", &account_id))
            .await
            .expect("installed manifest must resolve credentials");

        assert_eq!(resolved.secrets.slot_count(), 1);
    }

    /// Stored config outlives the manifest that authorised it, so the check has to
    /// run where resolution does and not only at the write path.
    #[tokio::test]
    async fn resolution_withholds_a_slot_the_installed_manifest_dropped() {
        let widget_type_id = Uuid::new_v4();
        let (store, account_id) =
            store_holding("a-1", credential::BuiltinType::BraiinsPool.id()).await;
        let (coordinator, _) = coordinator_declaring(widget_type_id, &[], store).await;

        let resolved = coordinator
            .resolve_credentials(&widget_bound_to(widget_type_id, "pool", &account_id))
            .await
            .expect("installed manifest must resolve credentials");

        assert_eq!(
            resolved.secrets.slot_count(),
            0,
            "a widget must not be handed a secret for a slot its manifest no longer declares"
        );
        assert!(resolved.view.is_empty());
    }

    /// An account whose type no longer matches the redeclared slot is the other half:
    /// the slot still exists, so only the type comparison can reject it.
    #[tokio::test]
    async fn resolution_withholds_a_slot_redeclared_with_another_type() {
        let widget_type_id = Uuid::new_v4();
        let (store, account_id) =
            store_holding("a-1", credential::BuiltinType::BraiinsPool.id()).await;
        let (coordinator, _) = coordinator_declaring(
            widget_type_id,
            &[("pool", credential::BuiltinType::GenericToken.id(), true)],
            store,
        )
        .await;

        let resolved = coordinator
            .resolve_credentials(&widget_bound_to(widget_type_id, "pool", &account_id))
            .await
            .expect("installed manifest must resolve credentials");

        assert_eq!(resolved.secrets.slot_count(), 0);
    }

    fn binding_violations(
        manifest: &bmc_widget_manifest::Manifest,
        bindings: &[(&str, &str)],
        accounts: &IndexMap<AccountId, Account>,
    ) -> Vec<tonic_types::FieldViolation> {
        let bindings = bindings
            .iter()
            .map(|(key, id)| ((*key).to_owned(), (*id).to_owned()))
            .collect();
        match validate_credential_bindings(manifest, &bindings, accounts) {
            Ok(_) => vec![],
            Err(violations) => violations.into(),
        }
    }

    #[test]
    fn credential_binding_of_a_matching_account_is_stored_typed() {
        let manifest =
            manifest_with_credentials(&[("pool", credential::BuiltinType::BraiinsPool.id(), true)]);
        let (id, account) = account_of_type("acct-1", credential::BuiltinType::BraiinsPool.id());
        let accounts = indexmap::indexmap! { id.clone() => account };

        let typed = validate_credential_bindings(
            &manifest,
            &[("pool".to_owned(), "acct-1".to_owned())]
                .into_iter()
                .collect(),
            &accounts,
        )
        .expect("BUG: a matching account must validate");

        assert_eq!(typed.len(), 1);
        assert_eq!(typed.values().next(), Some(&id));
    }

    #[test]
    fn credential_binding_of_an_unknown_account_is_rejected() {
        let manifest =
            manifest_with_credentials(&[("pool", credential::BuiltinType::BraiinsPool.id(), true)]);
        let violations = binding_violations(&manifest, &[("pool", "acct-gone")], &IndexMap::new());

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].field, r#"credential_bindings["pool"]"#);
        assert_eq!(violations[0].description, "Account not found");
    }

    #[test]
    fn credential_binding_of_a_wrong_type_account_is_rejected() {
        let manifest =
            manifest_with_credentials(&[("pool", credential::BuiltinType::BraiinsPool.id(), true)]);
        let (id, account) = account_of_type("acct-1", credential::BuiltinType::GenericToken.id());
        let accounts = indexmap::indexmap! { id => account };
        let violations = binding_violations(&manifest, &[("pool", "acct-1")], &accounts);

        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].description,
            "Account is of a different credential type"
        );
    }

    #[test]
    fn credential_binding_of_a_slot_the_manifest_does_not_declare_is_rejected() {
        let manifest = manifest_with_credentials(&[(
            "pool",
            credential::BuiltinType::BraiinsPool.id(),
            false,
        )]);
        let violations = binding_violations(&manifest, &[("mystery", "acct-1")], &IndexMap::new());

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].field, r#"credential_bindings["mystery"]"#);
        assert_eq!(violations[0].description, "Unknown credential slot");
    }

    #[test]
    fn an_empty_credential_binding_unbinds_the_slot() {
        let manifest =
            manifest_with_credentials(&[("pool", credential::BuiltinType::BraiinsPool.id(), true)]);
        let (id, account) = account_of_type("acct-1", credential::BuiltinType::BraiinsPool.id());
        let accounts = indexmap::indexmap! { id => account };

        let typed = validate_credential_bindings(
            &manifest,
            &[("pool".to_owned(), String::new())].into_iter().collect(),
            &accounts,
        )
        .expect("BUG: unbinding must validate");

        assert!(typed.is_empty(), "an empty id must not bind anything");
    }

    #[test]
    fn a_required_credential_slot_may_stay_unbound() {
        let manifest =
            manifest_with_credentials(&[("pool", credential::BuiltinType::BraiinsPool.id(), true)]);
        let typed = validate_credential_bindings(&manifest, &HashMap::new(), &IndexMap::new())
            .expect("BUG: a required slot must never block saving");

        assert!(typed.is_empty());
    }

    fn violation_count(
        manifest: &bmc_widget_manifest::Manifest,
        params: &web::WidgetDataStruct,
        mode: ValidateMode,
    ) -> usize {
        match validate_widget_params(manifest, params, mode) {
            Ok(_) => 0,
            Err(violations) => {
                let v: Vec<tonic_types::FieldViolation> = violations.into();
                v.len()
            }
        }
    }

    #[test]
    fn validate_widget_params_string_string_value_accepts() {
        let manifest = single_param_manifest(
            "color",
            bmc_widget_manifest::ParamKind::String {
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
            bmc_widget_manifest::ParamKind::String {
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
            bmc_widget_manifest::ParamKind::Integer {
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
            bmc_widget_manifest::ParamKind::Double {
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
            bmc_widget_manifest::ParamKind::Boolean {
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
            bmc_widget_manifest::ParamKind::String {
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
            bmc_widget_manifest::ParamKind::String {
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
            bmc_widget_manifest::ParamKind::Double {
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
            bmc_widget_manifest::ParamKind::Double {
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
            bmc_widget_manifest::ParamKind::Integer {
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
            bmc_widget_manifest::ParamKind::Integer {
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
            bmc_widget_manifest::ParamKind::Double {
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
            bmc_widget_manifest::ParamKind::String {
                format: None,
                enum_values: vec![
                    bmc_widget_manifest::StringOption {
                        value: "dark".into(),
                        label: "Dark".into(),
                    },
                    bmc_widget_manifest::StringOption {
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
            bmc_widget_manifest::ParamKind::String {
                format: None,
                enum_values: vec![
                    bmc_widget_manifest::StringOption {
                        value: "dark".into(),
                        label: "Dark".into(),
                    },
                    bmc_widget_manifest::StringOption {
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
            bmc_widget_manifest::ParamKind::Boolean {
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
            bmc_widget_manifest::ParamKind::Boolean {
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
            bmc_widget_manifest::ParamKind::Boolean {
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
    fn validate_widget_params_accumulates_per_field_violations() {
        let manifest = manifest_with_params(&[
            (
                "n",
                bmc_widget_manifest::ParamKind::Integer {
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
                bmc_widget_manifest::ParamKind::String {
                    format: None,
                    enum_values: vec![bmc_widget_manifest::StringOption {
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

        let violations = validate_widget_params(&manifest, &params, ValidateMode::Add)
            .expect_err("BUG: must reject both fields");
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
        use bmc_widget_manifest::{ParamDefinition, ParamKind, StringOption};
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
        use bmc_widget_manifest::{ParamDefinition, ParamKind};
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
        use bmc_widget_manifest::{ParamDefinition, ParamKind};
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
        use bmc_widget_manifest::{ParamDefinition, ParamKind};
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
        use bmc_widget_manifest::{ParamDefinition, ParamKind};
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
        params: BTreeMap<bmc_widget_manifest::ParamKey, bmc_widget_manifest::ParamValue>,
    ) -> crate::scene::Scene {
        crate::scene::Scene::fullscreen(widget_uid, params)
    }

    fn proto_scene_first_widget(proto: &web::Scene) -> &web::Widget {
        match proto.kind.as_ref().expect("BUG: kind") {
            web::scene::Kind::Fullscreen(f) => f.widget.as_ref().expect("BUG: widget"),
            web::scene::Kind::Combined(c) => c.widgets.first().expect("BUG: widget"),
        }
    }

    fn accounts_of(ids: &[&str]) -> IndexMap<AccountId, Account> {
        ids.iter()
            .map(|id| {
                let id = AccountId::from_str(id).expect("BUG: non-empty id");
                let account = Account {
                    id: id.clone(),
                    type_id: credential::BuiltinType::GenericToken.id().to_owned(),
                    name: "Token".to_owned(),
                    field_values: IndexMap::new(),
                    allow_hosts: Vec::new(),
                    created_at: chrono::Utc::now(),
                };
                (id, account)
            })
            .collect()
    }

    fn scene_bound_to(account: &str) -> scene::Scene {
        let mut scene = scene_with_widget(uuid::Uuid::new_v4(), BTreeMap::new());
        let slot = CredentialKey::try_new("pool".to_owned()).expect("BUG: valid key");
        let id = AccountId::from_str(account).expect("BUG: non-empty id");
        for widget in scene.widgets.values_mut() {
            widget.credential_bindings.insert(slot.clone(), id.clone());
        }

        scene
    }

    fn proto_bindings(proto: &web::Scene) -> &HashMap<String, String> {
        &proto_scene_first_widget(proto)
            .config
            .as_ref()
            .expect("BUG: config")
            .credential_bindings
            .as_ref()
            .expect("BUG: bindings")
            .bindings
    }

    #[test]
    fn scene_to_proto_emits_a_binding_whose_account_exists() {
        let proto = scene_to_proto(&scene_bound_to("acct-1"), &accounts_of(&["acct-1"]));

        assert_eq!(proto_bindings(&proto)["pool"], "acct-1");
    }

    #[test]
    fn scene_to_proto_omits_a_binding_whose_account_is_gone() {
        let proto = scene_to_proto(&scene_bound_to("acct-1"), &accounts_of(&[]));

        assert!(
            proto_bindings(&proto).is_empty(),
            "the editor must not be handed an id it would then fail to save"
        );
    }

    #[test]
    fn widget_info_to_proto_emits_declared_credential_slots() {
        use std::str::FromStr as _;

        let uid = uuid::Uuid::new_v4();
        let pool_type = credential::BuiltinType::BraiinsPool.id();
        let json = format!(
            r#"{{
                "uid": "{uid}",
                "version": "1.0.0",
                "name": "T",
                "description": "T",
                "binary": "bin/test",
                "credentials": {{
                    "pool": {{"type": "{pool_type}", "label": "Pool", "required": true}}
                }},
                "supported_viewports": [{{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}}]
            }}"#
        );
        let info = crate::widget::WidgetInfo::for_test(
            bmc_widget_manifest::Manifest::from_str(&json).expect("BUG: valid manifest"),
            std::path::PathBuf::from("/w"),
            std::path::PathBuf::from("/w/bin/test"),
            None,
        );

        let slots = widget_info_to_proto(&info, &bmc100_platform_descriptor()).credentials;

        assert_eq!(
            slots,
            vec![web::CredentialSlotDefinition {
                key: "pool".to_owned(),
                type_id: credential::BuiltinType::BraiinsPool.id().to_owned(),
                label: "Pool".to_owned(),
                description: None,
                required: true,
            }]
        );
    }

    #[test]
    fn scene_to_proto_emits_typed_params_directly() {
        use bmc_widget_manifest::{ParamKey, ParamValue as PV};
        use web::widget_data_value::Kind as VK;

        let widget_uid = uuid::Uuid::new_v4();
        let key = ParamKey::try_new("x".to_owned()).expect("BUG: valid key");
        let params: BTreeMap<ParamKey, PV> = [(key, PV::Integer(5))].into_iter().collect();

        let scene = scene_with_widget(widget_uid, params);
        let proto = scene_to_proto(&scene, &IndexMap::new());
        let widget = proto_scene_first_widget(&proto);
        let config = widget.config.as_ref().expect("BUG: config");
        let params = config.params.as_ref().expect("BUG: params");
        assert!(matches!(params.fields["x"].kind, Some(VK::IntegerValue(5))));
    }

    #[test]
    fn scene_to_proto_emits_each_param_value_arm() {
        use bmc_widget_manifest::{ParamKey, ParamValue as PV};
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
        let proto = scene_to_proto(&scene, &IndexMap::new());
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
    fn set_scene_cycling_accepts_slide_fade_and_none() {
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
        assert_eq!(
            parse_scene_cycling_transition(web::SceneCyclingTransition::None.into())
                .expect("BUG: None must parse"),
            SceneCyclingTransition::None,
        );
    }

    #[tokio::test]
    async fn set_scene_cycling_persists_and_applies_compositor_config() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir creation must succeed in tests");
        let config_path = tmp.path().join("bmc-config.json");
        let config_handle = Arc::new(RwLock::new(
            ConfigHandle::init(config_path, 50, 50, 50, 50, bmc_platform::Product::Bmc100)
                .await
                .0,
        ));
        let widget_manager = crate::widget::WidgetManager::init(Vec::new(), false).await;
        let widget_registry = widget_manager.registry();
        let compositor = Arc::new(RecordingCompositor::default());
        let compositor_for_coordinator: Arc<dyn crate::compositor::Compositor> = compositor.clone();
        let capabilities = bmc100_caps(None);
        let coordinator = Arc::new(Coordinator::new(
            widget_manager,
            compositor_for_coordinator,
            Some("test-display".to_owned()),
            Arc::clone(&widget_registry),
            capabilities,
            empty_secret_store().await,
        ));
        let (led_tx, _led_rx) = tokio::sync::mpsc::channel(16);
        let led_coordinator = crate::led_coordinator::spawn_led_coordinator(led_tx);
        let service = SceneManagementService::new(
            widget_registry,
            Arc::clone(&config_handle),
            empty_secret_store().await,
            coordinator,
            capabilities,
            led_coordinator,
        );

        let request = web::SetSceneCyclingRequest {
            scene_cycling: Some(web::SceneCycling {
                automatic_cycling_enabled: false,
                automatic_cycling_default_duration_sec: 12,
                transition: web::SceneCyclingTransition::Slide.into(),
            }),
        };

        service
            .set_scene_cycling(Request::new(request))
            .await
            .expect("BUG: valid scene cycling request must be accepted");

        let persisted = config_handle.read().await.scene_cycling();
        assert!(!persisted.automatic_cycling_enabled);
        assert_eq!(
            persisted.automatic_cycling_default_duration,
            Duration::from_secs(12),
        );
        assert_eq!(persisted.transition, SceneCyclingTransition::Slide);

        let applied = compositor
            .scene_cycling_configs
            .lock()
            .expect("BUG: recording compositor lock must not be poisoned");
        assert_eq!(applied.len(), 1);
        let applied = applied.first().expect("BUG: length asserted above");
        assert!(!applied.automatic_cycling_enabled);
        assert_eq!(
            applied.automatic_cycling_default_duration,
            Duration::from_secs(12),
        );
        assert_eq!(applied.transition, SceneCyclingTransition::Slide);
    }

    #[tokio::test]
    async fn set_scene_cycling_rejects_duration_below_minimum() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir creation must succeed in tests");
        let config_path = tmp.path().join("bmc-config.json");
        let config_handle = Arc::new(RwLock::new(
            ConfigHandle::init(config_path, 50, 50, 50, 50, bmc_platform::Product::Bmc100)
                .await
                .0,
        ));
        let widget_manager = crate::widget::WidgetManager::init(Vec::new(), false).await;
        let widget_registry = widget_manager.registry();
        let compositor = Arc::new(RecordingCompositor::default());
        let compositor_for_coordinator: Arc<dyn crate::compositor::Compositor> = compositor.clone();
        let capabilities = bmc100_caps(None);
        let coordinator = Arc::new(Coordinator::new(
            widget_manager,
            compositor_for_coordinator,
            Some("test-display".to_owned()),
            Arc::clone(&widget_registry),
            capabilities,
            empty_secret_store().await,
        ));
        let (led_tx, _led_rx) = tokio::sync::mpsc::channel(16);
        let led_coordinator = crate::led_coordinator::spawn_led_coordinator(led_tx);
        let service = SceneManagementService::new(
            widget_registry,
            Arc::clone(&config_handle),
            empty_secret_store().await,
            coordinator,
            capabilities,
            led_coordinator,
        );

        let below_minimum =
            u32::try_from((scene::Scene::MIN_CYCLE_DURATION.as_secs()).saturating_sub(1))
                .expect("BUG: minimum cycle duration fits in u32");
        let status = service
            .set_scene_cycling(Request::new(web::SetSceneCyclingRequest {
                scene_cycling: Some(web::SceneCycling {
                    automatic_cycling_enabled: true,
                    automatic_cycling_default_duration_sec: below_minimum,
                    transition: web::SceneCyclingTransition::Slide.into(),
                }),
            }))
            .await
            .expect_err("BUG: sub-minimum duration must be rejected");
        assert_eq!(status.code(), Code::InvalidArgument);
    }

    #[tokio::test]
    async fn update_scene_rejects_cycle_duration_below_minimum() {
        let widget_type_id = uuid::Uuid::new_v4();
        let tmp = tempfile::tempdir().expect("BUG: tempdir creation must succeed in tests");
        let config_path = tmp.path().join("bmc-config.json");
        let config_handle = Arc::new(RwLock::new(
            ConfigHandle::init(config_path, 50, 50, 50, 50, bmc_platform::Product::Bmc100)
                .await
                .0,
        ));
        let scene = scene::Scene::fullscreen(widget_type_id, BTreeMap::new());
        let scene_id = scene.id;
        {
            let mut config = config_handle.write().await;
            config.scenes_mut().clear();
            config.scenes_mut().insert(scene.id, scene);
        }

        let widget_manager = crate::widget::WidgetManager::init(Vec::new(), false).await;
        let widget_registry = widget_manager.registry();
        let compositor = Arc::new(RecordingCompositor::default());
        let compositor_for_coordinator: Arc<dyn crate::compositor::Compositor> = compositor.clone();
        let capabilities = bmc100_caps(None);
        let coordinator = Arc::new(Coordinator::new(
            widget_manager,
            compositor_for_coordinator,
            Some("test-display".to_owned()),
            Arc::clone(&widget_registry),
            capabilities,
            empty_secret_store().await,
        ));
        let (led_tx, _led_rx) = tokio::sync::mpsc::channel(16);
        let led_coordinator = crate::led_coordinator::spawn_led_coordinator(led_tx);
        let service = SceneManagementService::new(
            widget_registry,
            Arc::clone(&config_handle),
            empty_secret_store().await,
            coordinator,
            capabilities,
            led_coordinator,
        );

        let below_minimum =
            u32::try_from((scene::Scene::MIN_CYCLE_DURATION.as_secs()).saturating_sub(1))
                .expect("BUG: minimum cycle duration fits in u32");
        let status = service
            .update_scene(Request::new(web::UpdateSceneRequest {
                id: scene_id.to_string(),
                enabled: true,
                cycle_duration_sec: Some(below_minimum),
            }))
            .await
            .expect_err("BUG: sub-minimum cycle duration must be rejected");
        assert_eq!(status.code(), Code::InvalidArgument);
    }

    #[tokio::test]
    async fn refresh_compositor_scenes_pins_single_scene_during_preview() {
        let widget_type_id = uuid::Uuid::new_v4();
        let manifest = bmc_widget_manifest::Manifest {
            uid: widget_type_id,
            version: semver::Version::new(1, 0, 0),
            name: "test-widget".to_owned(),
            subname: None,
            description: "Test widget".to_owned(),
            config_help: None,
            author: None,
            binary: std::path::PathBuf::from("bin/widget"),
            icon: None,
            category: bmc_widget_manifest::WidgetCategory::Misc,
            settings: vec![],
            supported_viewports: vec![bmc_widget_manifest::WidgetViewportConstraint {
                viewport_shape: bmc_widget_manifest::ViewportShape::Rectangular,
                min_width: Some(1_280),
                max_width: Some(1_280),
                min_height: Some(480),
                max_height: Some(480),
                min_dpi: None,
                max_dpi: None,
            }],
            params: indexmap::IndexMap::new(),
            credentials: indexmap::IndexMap::new(),
        };
        let widget_registry = Arc::new(WidgetRegistry::new(vec![
            crate::widget::WidgetInfo::for_test(
                manifest,
                std::path::PathBuf::from("/test/widgets/test-widget"),
                std::path::PathBuf::from("/test/widgets/test-widget/bin/widget"),
                None,
            ),
        ]));

        let tmp = tempfile::tempdir().expect("BUG: tempdir creation must succeed in tests");
        let config_path = tmp.path().join("bmc-config.json");
        let config_handle = Arc::new(RwLock::new(
            ConfigHandle::init(config_path, 50, 50, 50, 50, bmc_platform::Product::Bmc100)
                .await
                .0,
        ));

        // Two enabled, supported scenes — without a preview both must cycle.
        let scene_a = scene::Scene::fullscreen(widget_type_id, BTreeMap::new());
        let scene_b = scene::Scene::fullscreen(widget_type_id, BTreeMap::new());
        let preview_id = scene_a.id;
        {
            let mut config = config_handle.write().await;
            config.scenes_mut().clear();
            config.scenes_mut().insert(scene_a.id, scene_a);
            config.scenes_mut().insert(scene_b.id, scene_b);
        }

        let widget_manager = crate::widget::WidgetManager::init(Vec::new(), false).await;
        let compositor = Arc::new(RecordingCompositor::default());
        let compositor_for_coordinator: Arc<dyn crate::compositor::Compositor> = compositor.clone();
        let capabilities = bmc100_caps(None);
        let coordinator = Arc::new(Coordinator::new(
            widget_manager,
            compositor_for_coordinator,
            Some("test-display".to_owned()),
            Arc::clone(&widget_registry),
            capabilities,
            empty_secret_store().await,
        ));
        let (led_tx, _led_rx) = tokio::sync::mpsc::channel(16);
        let led_coordinator = crate::led_coordinator::spawn_led_coordinator(led_tx);
        let service = SceneManagementService::new(
            widget_registry,
            Arc::clone(&config_handle),
            empty_secret_store().await,
            coordinator,
            capabilities,
            led_coordinator,
        );

        service.refresh_compositor_scenes().await;
        {
            let lists = compositor
                .scene_cycling_lists
                .lock()
                .expect("BUG: recording compositor lock must not be poisoned");
            let last = lists.last().expect("BUG: refresh must push a cycling list");
            assert_eq!(last.len(), 2, "without a preview both scenes must cycle");
        }

        *service.preview_scene_id.lock().await = Some(preview_id);
        service.refresh_compositor_scenes().await;
        {
            let lists = compositor
                .scene_cycling_lists
                .lock()
                .expect("BUG: recording compositor lock must not be poisoned");
            let last = lists.last().expect("BUG: preview must push a cycling list");
            assert_eq!(
                last.len(),
                1,
                "a held preview must pin exactly one scene so cycling cannot move off it",
            );
        }
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
        manifest: &bmc_widget_manifest::Manifest,
        params: &web::WidgetDataStruct,
    ) -> String {
        let violations = validate_widget_params(manifest, params, ValidateMode::Add)
            .expect_err("BUG: expected at least one violation");
        let v: Vec<tonic_types::FieldViolation> = violations.into();
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
            bmc_widget_manifest::ParamKind::String {
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
            bmc_widget_manifest::ParamKind::Integer {
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
            bmc_widget_manifest::ParamKind::Double {
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
            bmc_widget_manifest::ParamKind::Boolean {
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
            bmc_widget_manifest::ParamKind::Timezone {
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
            bmc_widget_manifest::ParamKind::Integer {
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
            bmc_widget_manifest::ParamKind::Integer {
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
            bmc_widget_manifest::ParamKind::Double {
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
            bmc_widget_manifest::ParamKind::Double {
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
            bmc_widget_manifest::ParamKind::Double {
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
            bmc_widget_manifest::ParamKind::String {
                format: None,
                enum_values: vec![bmc_widget_manifest::StringOption {
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
            bmc_widget_manifest::ParamKind::Integer {
                min: None,
                max: None,
                step: None,
                enum_values: vec![bmc_widget_manifest::IntegerOption {
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

    #[test]
    fn widget_size_maps_to_internal_placement() {
        assert_eq!(
            scene::WidgetPlacement::try_from(web::WidgetSize::Small),
            Ok(scene::WidgetPlacement::SlotSpan(scene::SlotSpan {
                columns: 1,
                rows: 1
            }))
        );
        assert_eq!(
            scene::WidgetPlacement::try_from(web::WidgetSize::Medium),
            Ok(scene::WidgetPlacement::SlotSpan(scene::SlotSpan {
                columns: 2,
                rows: 1
            }))
        );
        assert_eq!(
            scene::WidgetPlacement::try_from(web::WidgetSize::Large),
            Ok(scene::WidgetPlacement::SlotSpan(scene::SlotSpan {
                columns: 2,
                rows: 2
            }))
        );
        assert_eq!(
            scene::WidgetPlacement::try_from(web::WidgetSize::Full),
            Ok(scene::WidgetPlacement::Fullscreen)
        );
        assert!(scene::WidgetPlacement::try_from(web::WidgetSize::Unspecified).is_err());
    }

    #[test]
    fn internal_placement_maps_to_frontend_size_label() {
        assert_eq!(
            web::WidgetSize::try_from(&scene::WidgetPlacement::SlotSpan(scene::SlotSpan {
                columns: 1,
                rows: 1
            })),
            Ok(web::WidgetSize::Small)
        );
        assert_eq!(
            web::WidgetSize::try_from(&scene::WidgetPlacement::SlotSpan(scene::SlotSpan {
                columns: 2,
                rows: 1
            })),
            Ok(web::WidgetSize::Medium)
        );
        assert_eq!(
            web::WidgetSize::try_from(&scene::WidgetPlacement::SlotSpan(scene::SlotSpan {
                columns: 2,
                rows: 2
            })),
            Ok(web::WidgetSize::Large)
        );
        assert_eq!(
            web::WidgetSize::try_from(&scene::WidgetPlacement::Fullscreen),
            Ok(web::WidgetSize::Full)
        );
        assert!(
            web::WidgetSize::try_from(&scene::WidgetPlacement::SlotSpan(scene::SlotSpan {
                columns: 3,
                rows: 1
            }))
            .is_err()
        );
    }

    #[test]
    fn platform_descriptor_maps_fullscreen_size() {
        let desc = bmc100_platform_descriptor()
            .descriptor_for_placement(&scene::WidgetPlacement::Fullscreen)
            .expect("BUG: fullscreen must derive");
        assert_eq!(desc, bmc100_platform_descriptor().fullscreen);
    }

    #[test]
    fn platform_descriptor_maps_bmc100_slot_sizes() {
        let desc = bmc100_platform_descriptor()
            .descriptor_for_size(web::WidgetSize::Large)
            .expect("BUG: large must derive");
        assert_eq!(desc.width, 638);
        assert_eq!(desc.height, 480);
    }

    #[test]
    fn platform_descriptor_rejects_unknown_slot_span() {
        assert!(
            bmc100_platform_descriptor()
                .descriptor_for_placement(&scene::WidgetPlacement::SlotSpan(scene::SlotSpan {
                    columns: 3,
                    rows: 1
                }))
                .is_none()
        );
    }

    #[test]
    fn manifest_supported_sizes_are_calculated_from_constraints() {
        let constraints = vec![bmc_widget_manifest::WidgetViewportConstraint {
            viewport_shape: bmc_widget_manifest::ViewportShape::Rectangular,
            min_width: Some(638),
            max_width: Some(638),
            min_height: Some(480),
            max_height: Some(480),
            min_dpi: None,
            max_dpi: None,
        }];
        assert_eq!(
            supported_sizes_for_constraints(&bmc100_platform_descriptor(), &constraints),
            vec![web::WidgetSize::Large]
        );
    }

    #[test]
    fn bmc100_slot_size_requires_exact_descriptor_match() {
        let constraints = vec![bmc_widget_manifest::WidgetViewportConstraint {
            viewport_shape: bmc_widget_manifest::ViewportShape::Rectangular,
            min_width: Some(638),
            max_width: Some(638),
            min_height: Some(480),
            max_height: Some(480),
            min_dpi: Some(2),
            max_dpi: Some(2),
        }];
        assert!(
            !supported_sizes_for_constraints(&bmc100_platform_descriptor(), &constraints)
                .contains(&web::WidgetSize::Large)
        );
    }

    #[test]
    fn supported_sizes_include_fullscreen_when_full_descriptor_matches() {
        let constraints = vec![bmc_widget_manifest::WidgetViewportConstraint {
            viewport_shape: bmc_widget_manifest::ViewportShape::Rectangular,
            min_width: Some(1280),
            max_width: Some(1280),
            min_height: Some(480),
            max_height: Some(480),
            min_dpi: None,
            max_dpi: None,
        }];
        assert_eq!(
            supported_sizes_for_constraints(&bmc100_platform_descriptor(), &constraints),
            vec![web::WidgetSize::Full]
        );
    }

    use crate::compositor::{DisplayInfo, DisplayShape, HardwareCapabilities, SlotGrid};

    fn caps_with(
        grid: Option<SlotGrid>,
        display_shape: DisplayShape,
        width: u32,
        height: u32,
    ) -> HardwareCapabilities {
        HardwareCapabilities {
            display: DisplayInfo {
                width,
                height,
                shape: display_shape,
                dpi: 1,
            },
            slot_grid: grid,
        }
    }

    fn bmc100_caps(grid: Option<SlotGrid>) -> HardwareCapabilities {
        caps_with(grid, DisplayShape::Rectangular, 1_280, 480)
    }

    #[test]
    fn reject_combined_without_slot_grid_errors() {
        let err = reject_combined_when_no_slot_grid(&bmc100_caps(None))
            .expect_err("BUG: must reject combined ops without a slot grid");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    }

    #[test]
    fn reject_combined_with_slot_grid_passes() {
        assert!(
            reject_combined_when_no_slot_grid(&bmc100_caps(Some(SlotGrid {
                columns: 4,
                rows: 2
            })))
            .is_ok()
        );
    }

    #[test]
    fn widget_viewport_shape_from_caps_rectangular() {
        let caps = caps_with(None, DisplayShape::Rectangular, 320, 240);
        assert_eq!(
            widget_viewport_shape_from_caps(&caps),
            bmc_widget_manifest::ViewportShape::Rectangular,
        );
    }

    #[test]
    fn widget_viewport_shape_from_caps_round() {
        let caps = caps_with(None, DisplayShape::Round, 480, 480);
        assert_eq!(
            widget_viewport_shape_from_caps(&caps),
            bmc_widget_manifest::ViewportShape::Round,
        );
    }

    #[test]
    fn stamp_widget_viewport_shape_updates_fullscreen_scene_widget() {
        let caps = caps_with(None, DisplayShape::Round, 480, 480);
        let mut scene = scene::Scene::fullscreen(Uuid::new_v4(), BTreeMap::new());
        let widget = scene
            .widgets
            .values_mut()
            .next()
            .expect("BUG: Scene::fullscreen always contains one widget");
        stamp_widget_viewport_shape_from_caps(widget, &caps);
        assert_eq!(
            widget.viewport_shape,
            bmc_widget_manifest::ViewportShape::Round,
        );
    }

    #[test]
    fn supported_sizes_use_fullscreen_descriptor_from_capabilities() {
        let caps = caps_with(None, DisplayShape::Round, 480, 480);
        let constraints = vec![bmc_widget_manifest::WidgetViewportConstraint {
            viewport_shape: bmc_widget_manifest::ViewportShape::Round,
            min_width: Some(480),
            max_width: Some(480),
            min_height: Some(480),
            max_height: Some(480),
            min_dpi: None,
            max_dpi: None,
        }];
        let platform = PlatformDescriptor::from(&caps);
        assert_eq!(
            supported_sizes_for_constraints(&platform, &constraints),
            vec![web::WidgetSize::Full]
        );
    }

    #[test]
    fn slot_sizes_present_when_slot_grid_present() {
        let caps = bmc100_caps(Some(SlotGrid {
            columns: 4,
            rows: 2,
        }));
        let platform = PlatformDescriptor::from(&caps);
        assert!(
            !platform.slot_sizes.is_empty(),
            "slot_grid present → slot sizes emitted"
        );
    }

    #[test]
    fn slot_sizes_empty_when_slot_grid_absent() {
        let caps = caps_with(None, DisplayShape::Rectangular, 320, 240);
        let platform = PlatformDescriptor::from(&caps);
        assert!(platform.slot_sizes.is_empty());
    }

    #[test]
    fn widget_info_to_proto_sets_icon_url_only_with_icon() {
        use std::str::FromStr as _;

        let uid = uuid::Uuid::new_v4();
        let json = format!(
            r#"{{
                "uid": "{uid}",
                "version": "1.0.0",
                "name": "T",
                "description": "T",
                "binary": "bin/test",
                "supported_viewports": [{{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}}]
            }}"#
        );
        let manifest = bmc_widget_manifest::Manifest::from_str(&json).expect("BUG: valid manifest");

        let with_icon = crate::widget::WidgetInfo::for_test(
            manifest,
            std::path::PathBuf::from("/w"),
            std::path::PathBuf::from("/w/bin/test"),
            Some(std::path::PathBuf::from("/w/icon.svg")),
        );
        let proto = widget_info_to_proto(&with_icon, &bmc100_platform_descriptor());
        assert_eq!(proto.icon_url, Some(format!("/widgets/{uid}/icon")));
        assert_eq!(proto.subname, None, "no subname declared");

        let without_icon = crate::widget::WidgetInfo {
            icon_path: None,
            ..with_icon
        };
        let proto = widget_info_to_proto(&without_icon, &bmc100_platform_descriptor());
        assert_eq!(proto.icon_url, None);
    }

    #[test]
    fn widget_info_to_proto_passes_subname_through() {
        use std::str::FromStr as _;

        let json = format!(
            r#"{{
                "uid": "{}",
                "version": "1.0.0",
                "name": "T",
                "subname": "Analog",
                "description": "T",
                "binary": "bin/test",
                "supported_viewports": [{{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}}]
            }}"#,
            uuid::Uuid::new_v4()
        );
        let info = crate::widget::WidgetInfo::for_test(
            bmc_widget_manifest::Manifest::from_str(&json).expect("BUG: valid manifest"),
            std::path::PathBuf::from("/w"),
            std::path::PathBuf::from("/w/bin/test"),
            None,
        );
        let proto = widget_info_to_proto(&info, &bmc100_platform_descriptor());
        assert_eq!(proto.subname.as_deref(), Some("Analog"));
    }

    #[test]
    fn widget_info_to_proto_maps_category() {
        use std::str::FromStr as _;

        let make = |category_line: &str| {
            let json = format!(
                r#"{{
                    "uid": "{}",
                    "version": "1.0.0",
                    "name": "T",
                    "description": "T",
                    "binary": "bin/test",
                    {category_line}
                    "supported_viewports": [{{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}}]
                }}"#,
                uuid::Uuid::new_v4()
            );
            let info = crate::widget::WidgetInfo::for_test(
                bmc_widget_manifest::Manifest::from_str(&json).expect("BUG: valid manifest"),
                std::path::PathBuf::from("/w"),
                std::path::PathBuf::from("/w/bin/test"),
                None,
            );
            widget_info_to_proto(&info, &bmc100_platform_descriptor()).category
        };

        assert_eq!(
            make(r#""category": "space","#),
            i32::from(web::WidgetCategory::Space)
        );
        // Absent category defaults to misc through the manifest layer.
        assert_eq!(make(""), i32::from(web::WidgetCategory::Misc));
    }

    struct PreviewLifecycleFixture {
        _temp: tempfile::TempDir,
        service: Arc<SceneManagementService>,
        coordinator: Arc<Coordinator>,
        compositor: Arc<RecordingCompositor>,
        config: Arc<RwLock<ConfigHandle>>,
        scene_id: scene::SceneId,
        widget_uid: Uuid,
        account_id: AccountId,
    }

    async fn preview_lifecycle_fixture(
        enabled: bool,
        with_widget: bool,
    ) -> PreviewLifecycleFixture {
        let temp = tempfile::tempdir().expect("BUG: preview lifecycle tempdir");
        let package = temp.path().join("widget-package");
        std::fs::create_dir(&package).expect("BUG: create widget package");
        let widget_uid = Uuid::new_v4();
        std::fs::write(
            package.join("manifest.json"),
            format!(
                r#"{{"uid":"{widget_uid}","version":"1.0.0","name":"preview-test","description":"preview test","binary":"widget","supported_viewports":[{{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238}},{{"type":"rectangular","min_width":638,"max_width":638,"min_height":238,"max_height":238}}],"params":{{"label":{{"name":"Label","type":"string","default_value":"old"}}}},"credentials":{{"pool":{{"type":"braiins-pool","label":"Pool","required":true}}}}}}"#
            ),
        )
        .expect("BUG: write widget manifest");
        let binary = package.join("widget");
        std::fs::write(&binary, "#!/bin/sh\nexec sleep 30\n").expect("BUG: write widget binary");
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
            .expect("BUG: make widget executable");

        let manager =
            crate::widget::WidgetManager::init(vec![temp.path().to_path_buf()], false).await;
        let registry = manager.registry();
        let compositor = Arc::new(RecordingCompositor::default());
        let capabilities = bmc100_caps(Some(SlotGrid {
            columns: 4,
            rows: 2,
        }));
        let (secret_store, account_id) =
            store_holding("pool-account", credential::BuiltinType::BraiinsPool.id()).await;
        let coordinator = Arc::new(Coordinator::new(
            manager,
            Arc::clone(&compositor) as Arc<dyn crate::compositor::Compositor>,
            Some("test-display".to_owned()),
            Arc::clone(&registry),
            capabilities,
            Arc::clone(&secret_store),
        ));
        let config_path = temp.path().join("settings.json");
        let config = Arc::new(RwLock::new(
            ConfigHandle::init(config_path, 50, 50, 50, 50, bmc_platform::Product::Bmc100)
                .await
                .0,
        ));
        let mut configured = scene::Scene {
            id: scene::SceneId::generate(),
            enabled,
            cycle_duration: None,
            kind: scene::SceneKind::Combined,
            widgets: IndexMap::new(),
        };
        if with_widget {
            let label = serde_json::from_str("\"label\"").expect("BUG: valid parameter key");
            let widget = scene::Widget::new(
                widget_uid,
                [(
                    label,
                    bmc_widget_manifest::ParamValue::String("old".to_owned()),
                )]
                .into_iter()
                .collect(),
                scene::WidgetPosition { row: 0, col: 0 },
                scene::WidgetPlacement::SlotSpan(scene::SlotSpan {
                    columns: 1,
                    rows: 1,
                }),
            );
            configured.widgets.insert(widget.id, widget);
        }
        let scene_id = configured.id;
        config
            .write()
            .await
            .scenes_mut()
            .insert(scene_id, configured);
        let scenes_rx = config.read().await.subscribe_scenes_change();
        let accounts_rx = secret_store.read().await.subscribe_accounts_change();
        crate::widget::coordinator::start_credential_listener(
            Arc::clone(&coordinator),
            Arc::clone(&config),
            scenes_rx,
            accounts_rx,
        );
        let (led_tx, _led_rx) = tokio::sync::mpsc::channel(16);
        let service = Arc::new(SceneManagementService::new(
            registry,
            Arc::clone(&config),
            secret_store,
            Arc::clone(&coordinator),
            capabilities,
            crate::led_coordinator::spawn_led_coordinator(led_tx),
        ));
        PreviewLifecycleFixture {
            _temp: temp,
            service,
            coordinator,
            compositor,
            config,
            scene_id,
            widget_uid,
            account_id,
        }
    }

    fn add_widget_request(fixture: &PreviewLifecycleFixture) -> web::AddWidgetRequest {
        web::AddWidgetRequest {
            scene_id: fixture.scene_id.to_string(),
            position: Some(web::WidgetPosition { row: 0, col: 0 }),
            size: web::WidgetSize::Small.into(),
            config: Some(web::WidgetConfig {
                widget_uid: fixture.widget_uid.to_string(),
                params: Some(web::WidgetDataStruct::default()),
                credential_bindings: Some(web::CredentialBindings::default()),
            }),
        }
    }

    fn observe_registration_from_config_edit(fixture: &PreviewLifecycleFixture) {
        let config = Arc::clone(&fixture.config);
        let secret_store = Arc::clone(&fixture.service.secret_store);
        fixture.compositor.observe_next_registration(move || {
            assert!(
                config.try_read().is_err(),
                "registration must be enqueued before releasing the config write lock"
            );
            assert!(
                secret_store.try_write().is_err(),
                "registration must retain the account snapshot through enqueue"
            );
        });
    }

    async fn fixture_widget_id(fixture: &PreviewLifecycleFixture) -> scene::WidgetId {
        *fixture
            .config
            .read()
            .await
            .scenes()
            .get(&fixture.scene_id)
            .expect("BUG: fixture scene")
            .widgets
            .first()
            .expect("BUG: fixture widget")
            .0
    }

    fn update_widget_request(
        fixture: &PreviewLifecycleFixture,
        widget_id: scene::WidgetId,
    ) -> web::UpdateWidgetRequest {
        web::UpdateWidgetRequest {
            scene_id: fixture.scene_id.to_string(),
            id: widget_id.to_string(),
            position: Some(web::WidgetPosition { row: 0, col: 0 }),
            size: web::WidgetSize::Medium.into(),
            params: Some(fields_one("label", wdv_string("old"))),
            credential_bindings: Some(web::CredentialBindings::default()),
        }
    }

    fn hot_update_request(
        fixture: &PreviewLifecycleFixture,
        widget_id: scene::WidgetId,
        label: &str,
        binding: Option<&AccountId>,
    ) -> web::UpdateWidgetRequest {
        web::UpdateWidgetRequest {
            scene_id: fixture.scene_id.to_string(),
            id: widget_id.to_string(),
            position: Some(web::WidgetPosition { row: 0, col: 0 }),
            size: web::WidgetSize::Small.into(),
            params: Some(fields_one("label", wdv_string(label))),
            credential_bindings: Some(web::CredentialBindings {
                bindings: binding
                    .map(|account| {
                        [("pool".to_owned(), account.to_string())]
                            .into_iter()
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
        }
    }

    async fn start_fixture_widget(fixture: &PreviewLifecycleFixture) -> scene::WidgetId {
        fixture
            .coordinator
            .spawn_configured_scene_widgets(&fixture.config, fixture.scene_id)
            .await;
        wait_for_managed_widgets(&fixture.coordinator, 1).await;
        fixture_widget_id(fixture).await
    }

    async fn start_preview(
        fixture: &PreviewLifecycleFixture,
    ) -> BoxStream<'static, Result<(), Status>> {
        fixture
            .service
            .preview_scene(Request::new(fixture.scene_id.to_string()))
            .await
            .expect("BUG: preview must start")
            .into_inner()
    }

    async fn set_fixture_scene_enabled(fixture: &PreviewLifecycleFixture, enabled: bool) {
        fixture
            .service
            .update_scene(Request::new(web::UpdateSceneRequest {
                id: fixture.scene_id.to_string(),
                enabled,
                cycle_duration_sec: None,
            }))
            .await
            .expect("BUG: scene update must succeed");
    }

    async fn wait_for_preview_lock(service: &SceneManagementService) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while service.preview_scene_id.try_lock().is_ok() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the first lifecycle operation did not acquire the preview lock"
            );
            tokio::task::yield_now().await;
        }
    }

    async fn wait_for_preview_clear(service: &SceneManagementService) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            if service.preview_scene_id.lock().await.is_none() {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "preview teardown did not clear the active scene"
            );
            tokio::task::yield_now().await;
        }
    }

    async fn wait_for_managed_widgets(coordinator: &Coordinator, expected: usize) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            if coordinator.running_widget_count().await == expected {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "widget manager did not reach the expected running state"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    async fn wait_for_credential_attempts(compositor: &RecordingCompositor, expected: usize) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while compositor.credential_update_attempt_count() < expected {
            assert!(
                tokio::time::Instant::now() < deadline,
                "credential listener did not attempt the expected updates"
            );
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn binding_only_update_targets_once_and_equivalent_binding_is_a_noop() {
        let fixture = preview_lifecycle_fixture(true, true).await;
        let widget_id = start_fixture_widget(&fixture).await;
        let baseline_calls = fixture.compositor.widget_calls().len();

        fixture
            .service
            .update_widget(Request::new(hot_update_request(
                &fixture,
                widget_id,
                "old",
                Some(&fixture.account_id),
            )))
            .await
            .expect("binding update must succeed");

        fixture.compositor.wait_for_credential_push_count(1).await;

        {
            let pushes = fixture.compositor.credential_pushes();
            assert_eq!(pushes.len(), 1);
            assert_eq!(pushes[0].instance_id, widget_id.to_string());
            assert!(pushes[0].changed, "the new binding must change credentials");
        }
        assert!(
            fixture
                .compositor
                .parameter_pushes
                .lock()
                .expect("BUG: recording compositor lock must not be poisoned")
                .is_empty()
        );
        assert!(
            !fixture.compositor.widget_calls()[baseline_calls..]
                .iter()
                .any(|call| call.starts_with("deactivate ")
                    || call.starts_with("register_retained "))
        );

        fixture
            .service
            .update_widget(Request::new(hot_update_request(
                &fixture,
                widget_id,
                "old",
                Some(&fixture.account_id),
            )))
            .await
            .expect("equivalent update must succeed");
        fixture
            .service
            .secret_store
            .write()
            .await
            .save()
            .await
            .expect("BUG: listener barrier save must succeed");
        fixture.compositor.wait_for_credential_push_count(2).await;
        assert!(
            !fixture.compositor.credential_pushes()[1].changed,
            "an equivalent binding must not change credentials"
        );
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn binding_update_holds_config_while_waiting_for_secrets() {
        let fixture = preview_lifecycle_fixture(true, true).await;
        let widget_id = fixture_widget_id(&fixture).await;
        let secret_guard = fixture.service.secret_store.write().await;
        let service = Arc::clone(&fixture.service);
        let request = hot_update_request(&fixture, widget_id, "old", Some(&fixture.account_id));
        let update =
            tokio::spawn(async move { service.update_widget(Request::new(request)).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if fixture.config.try_read().is_err() {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("binding update must acquire config before waiting for secrets");

        assert!(
            tokio::time::timeout(Duration::from_millis(20), fixture.config.read())
                .await
                .is_err(),
            "a competing config reader must serialize behind binding update"
        );
        drop(secret_guard);
        update
            .await
            .expect("BUG: binding update task must not panic")
            .expect("BUG: binding update must converge after secrets are released");
        let _config_guard = tokio::time::timeout(Duration::from_secs(1), fixture.config.read())
            .await
            .expect("config lock must be released after binding update");
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn credential_listener_drops_source_locks_before_waiting_for_receipt() {
        let fixture = preview_lifecycle_fixture(true, true).await;
        let widget_id = start_fixture_widget(&fixture).await;
        fixture.compositor.hold_credential_receipts();
        let service = Arc::clone(&fixture.service);
        let request = hot_update_request(&fixture, widget_id, "old", Some(&fixture.account_id));
        let update =
            tokio::spawn(async move { service.update_widget(Request::new(request)).await });
        fixture.compositor.wait_for_credential_push_count(1).await;

        update
            .await
            .expect("BUG: update task")
            .expect("saved binding must not await credential delivery");
        assert!(
            fixture.config.try_write().is_ok(),
            "receipt wait must release configuration"
        );
        assert!(
            fixture.service.secret_store.try_write().is_ok(),
            "receipt wait must release accounts"
        );
        fixture.compositor.release_credential_receipts();
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn dropped_credential_receipt_does_not_block_later_refreshes() {
        let fixture = preview_lifecycle_fixture(true, true).await;
        let widget_id = start_fixture_widget(&fixture).await;
        fixture.compositor.hold_credential_receipts();
        let service = Arc::clone(&fixture.service);
        let request = hot_update_request(&fixture, widget_id, "old", Some(&fixture.account_id));
        let update =
            tokio::spawn(async move { service.update_widget(Request::new(request)).await });
        fixture.compositor.wait_for_credential_push_count(1).await;

        fixture.compositor.drop_credential_receipts();
        update
            .await
            .expect("BUG: update task")
            .expect("saved binding must not await credential delivery");
        fixture
            .service
            .secret_store
            .write()
            .await
            .save()
            .await
            .expect("BUG: listener barrier save must succeed");
        fixture.compositor.wait_for_credential_push_count(2).await;
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn retained_inactive_widget_accepts_params_and_credentials_while_detached() {
        let fixture = preview_lifecycle_fixture(true, true).await;
        let widget_id = start_fixture_widget(&fixture).await;
        set_fixture_scene_enabled(&fixture, false).await;
        wait_for_managed_widgets(&fixture.coordinator, 0).await;

        fixture
            .service
            .update_widget(Request::new(hot_update_request(
                &fixture,
                widget_id,
                "new",
                Some(&fixture.account_id),
            )))
            .await
            .expect("detached retained update must succeed");
        fixture.compositor.wait_for_credential_push_count(2).await;

        let key = WidgetInstanceKey::new(widget_id.as_uuid());
        assert_eq!(
            fixture.compositor.retained_mode(key),
            Some(crate::compositor::WidgetConnectionMode::Inactive)
        );
        assert_eq!(
            fixture
                .compositor
                .retained_params(key)
                .expect("retained params")["label"],
            serde_json::json!("new")
        );
        assert!(
            fixture.compositor.retained_credentials(key).is_some(),
            "detached record must retain resolved credentials"
        );
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn adding_widget_to_disabled_scene_registers_it_inactive() {
        let fixture = preview_lifecycle_fixture(false, false).await;
        observe_registration_from_config_edit(&fixture);

        let widget_id = fixture
            .service
            .add_widget(Request::new(add_widget_request(&fixture)))
            .await
            .expect("disabled widget add must succeed")
            .into_inner();
        let key = WidgetInstanceKey::new(
            Uuid::parse_str(&widget_id).expect("BUG: widget response must contain a UUID"),
        );

        assert_eq!(
            fixture.compositor.retained_mode(key),
            Some(crate::compositor::WidgetConnectionMode::Inactive)
        );
        assert_eq!(fixture.coordinator.running_widget_count().await, 0);
        let calls = fixture.compositor.widget_calls();
        assert!(
            calls
                .iter()
                .any(|call| call.starts_with("register_retained ")),
            "disabled addition must create a retained registration"
        );
        assert!(
            calls.iter().all(|call| !call.starts_with("activate ")),
            "disabled addition must not activate its retained registration"
        );
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn cloning_disabled_scene_registers_cloned_widgets_inactive() {
        let fixture = preview_lifecycle_fixture(false, true).await;
        observe_registration_from_config_edit(&fixture);

        let cloned_scene_id = fixture
            .service
            .clone_scene(Request::new(fixture.scene_id.to_string()))
            .await
            .expect("disabled scene clone must succeed")
            .into_inner();
        let cloned_scene_id = scene::SceneId::from(
            Uuid::parse_str(&cloned_scene_id).expect("BUG: clone response must contain a UUID"),
        );
        let cloned_widget_id = fixture.config.read().await.scenes()[&cloned_scene_id].widgets[0].id;
        let key = WidgetInstanceKey::new(cloned_widget_id.as_uuid());

        assert_eq!(
            fixture.compositor.retained_mode(key),
            Some(crate::compositor::WidgetConnectionMode::Inactive)
        );
        assert_eq!(fixture.coordinator.running_widget_count().await, 0);
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn disabled_geometry_update_refreshes_inactive_registration() {
        let fixture = preview_lifecycle_fixture(false, true).await;
        let widget_id = fixture_widget_id(&fixture).await;
        fixture
            .coordinator
            .spawn_configured_scene_widgets(&fixture.config, fixture.scene_id)
            .await;
        observe_registration_from_config_edit(&fixture);

        fixture
            .service
            .update_widget(Request::new(update_widget_request(&fixture, widget_id)))
            .await
            .expect("disabled geometry update must succeed");

        let key = WidgetInstanceKey::new(widget_id.as_uuid());
        let size = fixture
            .compositor
            .retained_size(key)
            .expect("disabled widget must retain its configured size");
        assert_eq!(
            size,
            crate::compositor::Size {
                width: 638,
                height: 238,
            }
        );
        assert_eq!(
            fixture.compositor.retained_mode(key),
            Some(crate::compositor::WidgetConnectionMode::Inactive)
        );
        assert_eq!(fixture.coordinator.running_widget_count().await, 0);
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn mixed_update_applies_params_while_credential_delivery_recovers() {
        let fixture = preview_lifecycle_fixture(true, true).await;
        let widget_id = start_fixture_widget(&fixture).await;
        fixture.compositor.fail_next_credential_update();

        fixture
            .service
            .update_widget(Request::new(hot_update_request(
                &fixture,
                widget_id,
                "new",
                Some(&fixture.account_id),
            )))
            .await
            .expect("saved update must not await credential delivery");
        wait_for_credential_attempts(&fixture.compositor, 1).await;
        fixture
            .service
            .secret_store
            .write()
            .await
            .save()
            .await
            .expect("BUG: credential retry wake must save");
        fixture.compositor.wait_for_credential_push_count(1).await;

        assert_eq!(
            fixture
                .compositor
                .retained_params(WidgetInstanceKey::new(widget_id.as_uuid()))
                .expect("retained params")["label"],
            serde_json::json!("new")
        );
        assert_eq!(
            fixture
                .compositor
                .parameter_pushes
                .lock()
                .expect("BUG: recording compositor lock must not be poisoned")
                .len(),
            1
        );
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn mixed_update_attempts_credentials_after_parameter_send_failure() {
        let fixture = preview_lifecycle_fixture(true, true).await;
        let widget_id = start_fixture_widget(&fixture).await;
        fixture.compositor.fail_next_parameter_update();

        let status = fixture
            .service
            .update_widget(Request::new(hot_update_request(
                &fixture,
                widget_id,
                "new",
                Some(&fixture.account_id),
            )))
            .await
            .expect_err("parameter send failure must reach the caller");

        assert_eq!(status.code(), tonic::Code::Internal);
        assert_eq!(fixture.compositor.credential_pushes().len(), 1);
        assert!(
            fixture
                .compositor
                .parameter_pushes
                .lock()
                .expect("BUG: recording compositor lock must not be poisoned")
                .is_empty()
        );
        fixture.coordinator.shutdown_widget_manager().await;
    }

    async fn add_during_preview_start(preview_first: bool) {
        let fixture = preview_lifecycle_fixture(false, false).await;
        let config_guard = fixture.config.write().await;
        let stream = if preview_first {
            let service = Arc::clone(&fixture.service);
            let scene_id = fixture.scene_id;
            let preview = tokio::spawn(async move {
                service
                    .preview_scene(Request::new(scene_id.to_string()))
                    .await
                    .expect("BUG: preview must start")
                    .into_inner()
            });
            wait_for_preview_lock(&fixture.service).await;
            let request = add_widget_request(&fixture);
            let service = Arc::clone(&fixture.service);
            let add = tokio::spawn(async move {
                service
                    .add_widget(Request::new(request))
                    .await
                    .expect("BUG: widget add must succeed")
            });
            drop(config_guard);
            let stream = preview.await.expect("BUG: preview task");
            add.await.expect("BUG: add task");
            stream
        } else {
            let request = add_widget_request(&fixture);
            let service = Arc::clone(&fixture.service);
            let add = tokio::spawn(async move {
                service
                    .add_widget(Request::new(request))
                    .await
                    .expect("BUG: widget add must succeed")
            });
            tokio::task::yield_now().await;
            let service = Arc::clone(&fixture.service);
            let scene_id = fixture.scene_id;
            let preview = tokio::spawn(async move {
                service
                    .preview_scene(Request::new(scene_id.to_string()))
                    .await
                    .expect("BUG: preview must start")
                    .into_inner()
            });
            wait_for_preview_lock(&fixture.service).await;
            drop(config_guard);
            add.await.expect("BUG: add task");
            preview.await.expect("BUG: preview task")
        };
        wait_for_managed_widgets(&fixture.coordinator, 1).await;
        assert_eq!(fixture.coordinator.running_widget_count().await, 1);
        let calls = fixture.compositor.widget_calls();
        assert!(calls.iter().any(|call| call.starts_with("activate ")));
        assert!(!calls.iter().any(|call| {
            call.starts_with("deactivate ") || call.starts_with("unregister_retained ")
        }));
        drop(stream);
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn add_widget_and_preview_start_converge_in_both_task_orders() {
        add_during_preview_start(true).await;
        add_during_preview_start(false).await;
    }

    async fn update_during_preview_start(preview_first: bool) {
        let fixture = preview_lifecycle_fixture(false, true).await;
        let widget_id = fixture_widget_id(&fixture).await;
        let config_guard = fixture.config.write().await;
        let stream = if preview_first {
            let service = Arc::clone(&fixture.service);
            let scene_id = fixture.scene_id;
            let preview = tokio::spawn(async move {
                service
                    .preview_scene(Request::new(scene_id.to_string()))
                    .await
                    .expect("BUG: preview must start")
                    .into_inner()
            });
            wait_for_preview_lock(&fixture.service).await;
            let service = Arc::clone(&fixture.service);
            let request = update_widget_request(&fixture, widget_id);
            let update = tokio::spawn(async move {
                service
                    .update_widget(Request::new(request))
                    .await
                    .expect("BUG: widget update must succeed")
            });
            drop(config_guard);
            let stream = preview.await.expect("BUG: preview task");
            update.await.expect("BUG: update task");
            stream
        } else {
            let service = Arc::clone(&fixture.service);
            let request = update_widget_request(&fixture, widget_id);
            let update = tokio::spawn(async move {
                service
                    .update_widget(Request::new(request))
                    .await
                    .expect("BUG: widget update must succeed")
            });
            wait_for_preview_lock(&fixture.service).await;
            let service = Arc::clone(&fixture.service);
            let scene_id = fixture.scene_id;
            let preview = tokio::spawn(async move {
                service
                    .preview_scene(Request::new(scene_id.to_string()))
                    .await
                    .expect("BUG: preview must start")
                    .into_inner()
            });
            drop(config_guard);
            update.await.expect("BUG: update task");
            preview.await.expect("BUG: preview task")
        };

        wait_for_managed_widgets(&fixture.coordinator, 1).await;
        let retained_size = fixture
            .compositor
            .retained_size(WidgetInstanceKey::new(widget_id.as_uuid()))
            .expect("updated widget must remain registered");
        assert_eq!(
            retained_size,
            crate::compositor::Size {
                width: 638,
                height: 238
            }
        );
        drop(stream);
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn restart_update_and_preview_start_converge_in_both_lock_orders() {
        update_during_preview_start(true).await;
        update_during_preview_start(false).await;
    }

    #[tokio::test]
    async fn preview_pin_reads_layout_after_widget_start_receipts() {
        let fixture = preview_lifecycle_fixture(false, true).await;
        fixture.compositor.hold_widget_receipts();
        let service = Arc::clone(&fixture.service);
        let scene_id = fixture.scene_id;
        let preview = tokio::spawn(async move {
            service
                .preview_scene(Request::new(scene_id.to_string()))
                .await
                .expect("BUG: preview must start")
                .into_inner()
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while !fixture
            .compositor
            .widget_calls()
            .iter()
            .any(|call| call.starts_with("activate "))
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "preview start did not enqueue widget activation"
            );
            tokio::task::yield_now().await;
        }

        {
            let mut config = tokio::time::timeout(Duration::from_secs(1), fixture.config.write())
                .await
                .expect("receipt wait must release the configuration lock");
            let widget = config
                .scenes_mut()
                .get_mut(&fixture.scene_id)
                .expect("BUG: fixture scene")
                .widgets
                .first_mut()
                .expect("BUG: fixture widget")
                .1;
            widget.placement = scene::WidgetPlacement::SlotSpan(scene::SlotSpan {
                columns: 2,
                rows: 1,
            });
        }
        fixture.compositor.release_widget_receipts();
        let stream = preview.await.expect("BUG: preview task");

        {
            let lists = fixture
                .compositor
                .scene_cycling_lists
                .lock()
                .expect("BUG: recording compositor lock must not be poisoned");
            let pinned = lists.last().expect("preview must pin a scene");
            assert_eq!(pinned.len(), 1, "preview must pin exactly one scene");
            assert_eq!(
                pinned[0].widgets[0].size,
                crate::compositor::Size {
                    width: 638,
                    height: 238,
                },
                "preview pin must include layout changes made during widget startup"
            );
        }
        drop(stream);
        wait_for_preview_clear(&fixture.service).await;
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn cancelled_preview_start_releases_slot() {
        let fixture = preview_lifecycle_fixture(false, true).await;
        fixture.compositor.hold_widget_receipts();
        let service = Arc::clone(&fixture.service);
        let scene_id = fixture.scene_id;
        let preview = tokio::spawn(async move {
            service
                .preview_scene(Request::new(scene_id.to_string()))
                .await
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while !fixture
            .compositor
            .widget_calls()
            .iter()
            .any(|call| call.starts_with("activate "))
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "preview start did not enqueue widget activation"
            );
            tokio::task::yield_now().await;
        }

        preview.abort();
        let Err(error) = preview.await else {
            panic!("BUG: preview task must report cancellation");
        };
        assert!(error.is_cancelled(), "preview task must be cancelled");
        fixture.compositor.release_widget_receipts();
        wait_for_preview_clear(&fixture.service).await;

        let stream = start_preview(&fixture).await;
        wait_for_managed_widgets(&fixture.coordinator, 1).await;
        drop(stream);
        wait_for_preview_clear(&fixture.service).await;
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn preview_teardown_orders_reopen_after_restoration() {
        let fixture = preview_lifecycle_fixture(false, true).await;
        let stream = start_preview(&fixture).await;
        wait_for_managed_widgets(&fixture.coordinator, 1).await;
        fixture.compositor.hold_widget_receipts();
        drop(stream);

        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while !fixture
            .compositor
            .widget_calls()
            .iter()
            .any(|call| call.starts_with("deactivate "))
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "preview teardown did not enqueue widget deactivation"
            );
            tokio::task::yield_now().await;
        }
        tokio::task::yield_now().await;

        assert!(
            fixture.service.preview_scene_id.try_lock().is_err(),
            "preview teardown must reserve the slot until restoration is enqueued"
        );
        let service = Arc::clone(&fixture.service);
        let scene_id = fixture.scene_id;
        let reopen = tokio::spawn(async move {
            service
                .preview_scene(Request::new(scene_id.to_string()))
                .await
                .expect("BUG: preview must reopen")
                .into_inner()
        });
        tokio::task::yield_now().await;
        assert!(!reopen.is_finished(), "preview reopened before restoration");
        fixture.compositor.release_widget_receipts();
        let reopened_stream = reopen.await.expect("BUG: preview reopen task");
        {
            let lists = fixture
                .compositor
                .scene_cycling_lists
                .lock()
                .expect("BUG: recording compositor lock must not be poisoned");
            assert_eq!(
                lists.last().map(Vec::len),
                Some(1),
                "reopened preview pin must follow the old cycling restoration"
            );
        }
        drop(reopened_stream);
        wait_for_preview_clear(&fixture.service).await;
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn update_survives_enabled_scene_handoff_to_preview() {
        let fixture = preview_lifecycle_fixture(true, true).await;
        fixture
            .coordinator
            .spawn_configured_scene_widgets(&fixture.config, fixture.scene_id)
            .await;
        wait_for_managed_widgets(&fixture.coordinator, 1).await;
        let stream = start_preview(&fixture).await;
        let widget_id = fixture_widget_id(&fixture).await;

        fixture.compositor.hold_widget_receipts();
        let service = Arc::clone(&fixture.service);
        let request = update_widget_request(&fixture, widget_id);
        let update = tokio::spawn(async move {
            service
                .update_widget(Request::new(request))
                .await
                .expect("BUG: widget update must succeed")
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while !fixture
            .compositor
            .widget_calls()
            .iter()
            .any(|call| call.starts_with("deactivate "))
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "replacement did not enqueue its cutoff"
            );
            tokio::task::yield_now().await;
        }

        set_fixture_scene_enabled(&fixture, false).await;
        fixture.compositor.release_widget_receipts();
        update.await.expect("BUG: update task");
        wait_for_managed_widgets(&fixture.coordinator, 1).await;
        let retained_size = fixture
            .compositor
            .retained_size(WidgetInstanceKey::new(widget_id.as_uuid()))
            .expect("updated widget must remain registered");
        assert_eq!(
            retained_size,
            crate::compositor::Size {
                width: 638,
                height: 238
            }
        );

        drop(stream);
        wait_for_preview_clear(&fixture.service).await;
        wait_for_managed_widgets(&fixture.coordinator, 0).await;
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn cancelled_replacement_update_still_starts_the_saved_successor() {
        let fixture = preview_lifecycle_fixture(true, true).await;
        fixture
            .coordinator
            .spawn_configured_scene_widgets(&fixture.config, fixture.scene_id)
            .await;
        wait_for_managed_widgets(&fixture.coordinator, 1).await;
        let widget_id = fixture_widget_id(&fixture).await;

        fixture.compositor.hold_widget_receipts();
        let service = Arc::clone(&fixture.service);
        let request = update_widget_request(&fixture, widget_id);
        let request_task =
            tokio::spawn(async move { service.update_widget(Request::new(request)).await });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while !fixture
            .compositor
            .widget_calls()
            .iter()
            .any(|call| call.starts_with("deactivate "))
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "saved replacement did not enqueue its cutoff"
            );
            tokio::task::yield_now().await;
        }

        request_task.abort();
        fixture.compositor.release_widget_receipts();
        wait_for_managed_widgets(&fixture.coordinator, 1).await;
        let key = WidgetInstanceKey::new(widget_id.as_uuid());
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            let retained_size = fixture.compositor.retained_size(key);
            if retained_size
                == Some(crate::compositor::Size {
                    width: 638,
                    height: 238,
                })
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "detached update did not register the saved successor"
            );
            tokio::task::yield_now().await;
        }
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn enabling_during_preview_keeps_widget_running_after_stream_drop() {
        let fixture = preview_lifecycle_fixture(false, true).await;
        let stream = start_preview(&fixture).await;
        wait_for_managed_widgets(&fixture.coordinator, 1).await;

        set_fixture_scene_enabled(&fixture, true).await;
        drop(stream);
        wait_for_preview_clear(&fixture.service).await;
        wait_for_managed_widgets(&fixture.coordinator, 1).await;

        set_fixture_scene_enabled(&fixture, false).await;
        wait_for_managed_widgets(&fixture.coordinator, 0).await;
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn enabling_during_preview_recovers_a_missing_widget() {
        let fixture = preview_lifecycle_fixture(false, true).await;
        let stream = start_preview(&fixture).await;
        wait_for_managed_widgets(&fixture.coordinator, 1).await;
        let widget_id = fixture_widget_id(&fixture).await;
        fixture
            .coordinator
            .enqueue_widget_replacement(WidgetInstanceKey::new(widget_id.as_uuid()))
            .await
            .wait()
            .await;
        wait_for_managed_widgets(&fixture.coordinator, 0).await;

        set_fixture_scene_enabled(&fixture, true).await;
        wait_for_managed_widgets(&fixture.coordinator, 1).await;

        drop(stream);
        wait_for_preview_clear(&fixture.service).await;
        wait_for_managed_widgets(&fixture.coordinator, 1).await;
        set_fixture_scene_enabled(&fixture, false).await;
        fixture.coordinator.shutdown_widget_manager().await;
    }

    #[tokio::test]
    async fn disabling_during_preview_defers_stop_until_stream_drop() {
        let fixture = preview_lifecycle_fixture(true, true).await;
        fixture
            .coordinator
            .spawn_configured_scene_widgets(&fixture.config, fixture.scene_id)
            .await;
        wait_for_managed_widgets(&fixture.coordinator, 1).await;
        let stream = start_preview(&fixture).await;

        set_fixture_scene_enabled(&fixture, false).await;
        wait_for_managed_widgets(&fixture.coordinator, 1).await;
        drop(stream);
        wait_for_preview_clear(&fixture.service).await;
        wait_for_managed_widgets(&fixture.coordinator, 0).await;
        fixture.coordinator.shutdown_widget_manager().await;
    }
}
