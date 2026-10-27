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

//! Typed widget manifests, so a test spells out only the params it exercises.

use std::path::{Path, PathBuf};

use bmc_widget_manifest::{
    ArrayParam, EnumControl, IntegerParam, ListItems, Manifest, ParamDefinition, ParamKey,
    ParamKind, ParamValue, Programs, ScalarKind, StringParam, ViewportShape, WidgetCategory,
    WidgetViewportConstraint,
};
use indexmap::IndexMap;
use uuid::Uuid;

/// A package without params that runs `widget` in exactly a `width`×`height` rectangle.
pub(crate) fn widget_manifest(
    uid: Uuid,
    version: semver::Version,
    width: u32,
    height: u32,
) -> Manifest {
    Manifest {
        uid,
        version,
        name: "T".to_owned(),
        subname: None,
        description: "T".to_owned(),
        config_help: None,
        author: None,
        binary: PathBuf::from("widget"),
        icon: None,
        category: WidgetCategory::Misc,
        settings: vec![],
        supported_viewports: vec![exact_viewport(width, height)],
        params: IndexMap::new(),
        credentials: IndexMap::new(),
        programs: Programs::default(),
    }
}

pub(crate) fn exact_viewport(width: u32, height: u32) -> WidgetViewportConstraint {
    WidgetViewportConstraint {
        viewport_shape: ViewportShape::Rectangular,
        min_width: Some(width),
        max_width: Some(width),
        min_height: Some(height),
        max_height: Some(height),
        min_dpi: None,
        max_dpi: None,
    }
}

pub(crate) fn write_manifest(dir: &Path, manifest: &Manifest) {
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_string(manifest).expect("BUG: a manifest serializes"),
    )
    .expect("BUG: write manifest");
}

pub(crate) fn param_key(name: &str) -> ParamKey {
    ParamKey::try_new(name.to_owned()).expect("BUG: a valid test key")
}

pub(crate) fn string_param(default: &str) -> ParamDefinition {
    required(ParamKind::String(plain_string(Some(default.to_owned()))))
}

pub(crate) fn integer_param(default: i32) -> ParamDefinition {
    required(ParamKind::Integer(IntegerParam {
        min: None,
        max: None,
        step: None,
        unit: None,
        enum_values: vec![],
        enum_control: EnumControl::default(),
        default_value: Some(default),
        placeholder: None,
    }))
}

pub(crate) fn string_list_param(defaults: &[&str]) -> ParamDefinition {
    required(ParamKind::Array(ArrayParam {
        items: ListItems::Scalar {
            kind: ScalarKind::String(plain_string(None)),
            unique: false,
        },
        min_items: 1,
        max_items: 8,
        default_value: defaults
            .iter()
            .map(|item| ParamValue::String((*item).to_owned()))
            .collect(),
    }))
}

fn required(kind: ParamKind) -> ParamDefinition {
    ParamDefinition {
        name: "T".to_owned(),
        description: None,
        is_optional: false,
        kind,
    }
}

fn plain_string(default_value: Option<String>) -> StringParam {
    StringParam {
        format: None,
        min_length: None,
        max_length: None,
        enum_values: vec![],
        enum_control: EnumControl::default(),
        default_value,
        placeholder: None,
    }
}
